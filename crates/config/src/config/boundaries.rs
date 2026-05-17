//! Architecture boundary zone and rule definitions.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use globset::Glob;
use rustc_hash::FxHashSet;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Process-local dedup state for the
/// `patterns + autoDiscover` footgun warning. Keyed on the offending zone
/// name. The warn fires once per (process, zone name) so long-running hosts
/// (`fallow watch`, the LSP, the NAPI worker, the MCP server) do not spam
/// the same diagnostic on every re-analysis. Restart re-arms the warning.
static AUTO_DISCOVER_PATTERNS_WARN_SEEN: OnceLock<Mutex<FxHashSet<String>>> = OnceLock::new();

/// Returns `true` if the warn for `zone_name` has not yet fired in this
/// process, `false` if it has already fired. A poisoned mutex falls back to
/// "would fire" so the user still sees one diagnostic per session.
fn record_auto_discover_patterns_warn_seen(zone_name: &str) -> bool {
    let seen = AUTO_DISCOVER_PATTERNS_WARN_SEEN.get_or_init(|| Mutex::new(FxHashSet::default()));
    seen.lock()
        .map_or(true, |mut set| set.insert(zone_name.to_owned()))
}

/// Built-in architecture presets.
///
/// Each preset expands into a set of zones and import rules for a common
/// architecture pattern. User-defined zones and rules merge on top of the
/// preset defaults (zones with the same name replace the preset zone;
/// rules with the same `from` replace the preset rule).
///
/// # Examples
///
/// ```
/// use fallow_config::BoundaryPreset;
///
/// let preset: BoundaryPreset = serde_json::from_str(r#""layered""#).unwrap();
/// assert!(matches!(preset, BoundaryPreset::Layered));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum BoundaryPreset {
    /// Classic layered architecture: presentation → application → domain ← infrastructure.
    /// Infrastructure may also import from application (common in DI frameworks).
    Layered,
    /// Hexagonal / ports-and-adapters: adapters → ports → domain.
    Hexagonal,
    /// Feature-Sliced Design: app > pages > widgets > features > entities > shared.
    /// Each layer may only import from layers below it.
    FeatureSliced,
    /// Bulletproof React: app → features → shared + server.
    /// Feature modules are isolated from each other via `autoDiscover`: every
    /// immediate child of `src/features/` becomes its own `features/<name>` zone,
    /// and cross-feature imports are reported as boundary violations.
    ///
    /// **Trade-off (intentional):** top-level files in `src/features/` (e.g.
    /// `src/features/index.ts` barrel, `src/features/types.ts`) do NOT match any
    /// child pattern and are unclassified, meaning they are unrestricted by the
    /// preset. This is deliberate so feature barrels can re-export children
    /// without producing false-positive `features → features/<child>` violations.
    /// To classify top-level files strictly, override the `features` zone with
    /// an explicit user definition that includes a `patterns` field.
    Bulletproof,
}

impl BoundaryPreset {
    /// Expand the preset into default zones and rules.
    ///
    /// `source_root` is the directory prefix for zone patterns (e.g., `"src"`, `"lib"`).
    /// Patterns are generated as `{source_root}/{zone_name}/**`.
    #[must_use]
    pub fn default_config(&self, source_root: &str) -> (Vec<BoundaryZone>, Vec<BoundaryRule>) {
        match self {
            Self::Layered => Self::layered_config(source_root),
            Self::Hexagonal => Self::hexagonal_config(source_root),
            Self::FeatureSliced => Self::feature_sliced_config(source_root),
            Self::Bulletproof => Self::bulletproof_config(source_root),
        }
    }

    fn zone(name: &str, source_root: &str) -> BoundaryZone {
        BoundaryZone {
            name: name.to_owned(),
            patterns: vec![format!("{source_root}/{name}/**")],
            auto_discover: vec![],
            root: None,
        }
    }

    fn rule(from: &str, allow: &[&str]) -> BoundaryRule {
        BoundaryRule {
            from: from.to_owned(),
            allow: allow.iter().map(|s| (*s).to_owned()).collect(),
            allow_type_only: Vec::new(),
        }
    }

    fn layered_config(source_root: &str) -> (Vec<BoundaryZone>, Vec<BoundaryRule>) {
        let zones = vec![
            Self::zone("presentation", source_root),
            Self::zone("application", source_root),
            Self::zone("domain", source_root),
            Self::zone("infrastructure", source_root),
        ];
        let rules = vec![
            Self::rule("presentation", &["application"]),
            Self::rule("application", &["domain"]),
            Self::rule("domain", &[]),
            Self::rule("infrastructure", &["domain", "application"]),
        ];
        (zones, rules)
    }

    fn hexagonal_config(source_root: &str) -> (Vec<BoundaryZone>, Vec<BoundaryRule>) {
        let zones = vec![
            Self::zone("adapters", source_root),
            Self::zone("ports", source_root),
            Self::zone("domain", source_root),
        ];
        let rules = vec![
            Self::rule("adapters", &["ports"]),
            Self::rule("ports", &["domain"]),
            Self::rule("domain", &[]),
        ];
        (zones, rules)
    }

    fn feature_sliced_config(source_root: &str) -> (Vec<BoundaryZone>, Vec<BoundaryRule>) {
        let layer_names = ["app", "pages", "widgets", "features", "entities", "shared"];
        let zones = layer_names
            .iter()
            .map(|name| Self::zone(name, source_root))
            .collect();
        let rules = layer_names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let below: Vec<&str> = layer_names[i + 1..].to_vec();
                Self::rule(name, &below)
            })
            .collect();
        (zones, rules)
    }

    fn bulletproof_config(source_root: &str) -> (Vec<BoundaryZone>, Vec<BoundaryRule>) {
        let zones = vec![
            Self::zone("app", source_root),
            BoundaryZone {
                // `features` is a logical group only: auto-discovered child
                // zones (`features/<name>`) classify the actual files. Leaving
                // `patterns` empty keeps top-level files in `src/features/`
                // (typically a barrel like `src/features/index.ts`) unclassified
                // so the barrel can re-export children without a cross-zone
                // `features → features/<child>` false positive.
                name: "features".to_owned(),
                patterns: vec![],
                auto_discover: vec![format!("{source_root}/features")],
                root: None,
            },
            BoundaryZone {
                name: "shared".to_owned(),
                patterns: [
                    "components",
                    "hooks",
                    "lib",
                    "utils",
                    "utilities",
                    "providers",
                    "shared",
                    "types",
                    "styles",
                    "i18n",
                ]
                .iter()
                .map(|dir| format!("{source_root}/{dir}/**"))
                .collect(),
                auto_discover: vec![],
                root: None,
            },
            Self::zone("server", source_root),
        ];
        let rules = vec![
            Self::rule("app", &["features", "shared", "server"]),
            Self::rule("features", &["shared", "server"]),
            Self::rule("server", &["shared"]),
            Self::rule("shared", &[]),
        ];
        (zones, rules)
    }
}

/// Architecture boundary configuration.
///
/// Defines zones (directory groupings) and rules (which zones may import from which).
/// Optionally uses a built-in preset as a starting point.
///
/// # Examples
///
/// ```
/// use fallow_config::BoundaryConfig;
///
/// let json = r#"{
///     "zones": [
///         { "name": "ui", "patterns": ["src/components/**"] },
///         { "name": "db", "patterns": ["src/db/**"] }
///     ],
///     "rules": [
///         { "from": "ui", "allow": ["db"] }
///     ]
/// }"#;
/// let config: BoundaryConfig = serde_json::from_str(json).unwrap();
/// assert_eq!(config.zones.len(), 2);
/// assert_eq!(config.rules.len(), 1);
/// ```
///
/// Using a preset:
///
/// ```
/// use fallow_config::BoundaryConfig;
///
/// let json = r#"{ "preset": "layered" }"#;
/// let mut config: BoundaryConfig = serde_json::from_str(json).unwrap();
/// config.expand("src");
/// assert_eq!(config.zones.len(), 4);
/// assert_eq!(config.rules.len(), 4);
/// ```
#[derive(Debug, Default, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryConfig {
    /// Built-in architecture preset. When set, expands into default zones and rules.
    /// User-defined zones and rules merge on top: zones with the same name replace
    /// the preset zone; rules with the same `from` replace the preset rule.
    /// Preset patterns use `{rootDir}/{zone}/**` where rootDir is auto-detected
    /// from tsconfig.json (falls back to `src`).
    /// Note: preset patterns are flat (`src/<zone>/**`). For monorepos with
    /// per-package source directories, define zones explicitly instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<BoundaryPreset>,
    /// Named zones mapping directory patterns to architectural layers.
    #[serde(default)]
    pub zones: Vec<BoundaryZone>,
    /// Import rules between zones. A zone with a rule entry can only import
    /// from the listed zones (plus itself). A zone without a rule entry is unrestricted.
    #[serde(default)]
    pub rules: Vec<BoundaryRule>,
}

/// A named zone grouping files by directory pattern.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryZone {
    /// Zone identifier referenced in rules (e.g., `"ui"`, `"database"`, `"shared"`).
    pub name: String,
    /// Glob patterns (relative to project root) that define zone membership.
    /// A file belongs to the first zone whose pattern matches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
    /// Directories whose immediate child directories should become separate
    /// zones under this logical group.
    ///
    /// For example, `{ "name": "features", "autoDiscover": ["src/features"] }`
    /// creates zones such as `features/auth` and `features/billing`, each with
    /// a pattern for its own subtree. Rules that reference `features` expand to
    /// every discovered child zone. If `patterns` is also set, the parent zone
    /// remains as a fallback after discovered child zones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auto_discover: Vec<String>,
    /// Optional subtree scope for monorepo per-package boundaries.
    ///
    /// When set, the zone's `patterns` are matched against paths *relative*
    /// to this directory rather than the project root. At classification
    /// time, fallow checks that a candidate path starts with `root` and
    /// strips that prefix before glob-matching the patterns against the
    /// remainder. Files outside the subtree never match the zone.
    ///
    /// Useful for monorepos where each package has the same internal
    /// directory layout: instead of writing `packages/app/src/**` and
    /// `packages/core/src/**` (which collide on shared zone names), set
    /// `root: "packages/app/"` and `patterns: ["src/**"]` per package.
    ///
    /// Trailing slash and leading `./` are normalized; backslashes are
    /// converted to forward slashes. Patterns must NOT redundantly include
    /// the root prefix: `root: "packages/app/"` with
    /// `patterns: ["packages/app/src/**"]` is rejected with
    /// `FALLOW-BOUNDARY-ROOT-REDUNDANT-PREFIX` because patterns are
    /// resolved relative to the root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

/// An import rule between zones.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryRule {
    /// The zone this rule applies to (the importing side).
    pub from: String,
    /// Zones that `from` is allowed to import from. Self-imports are always allowed.
    /// An empty list means the zone may not import from any other zone.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Zones that `from` may type-only-import from even when not listed in
    /// `allow`. Mirrors the `allow` shape: a list of target zone names. A
    /// type-only import declaration (`import type {...}`, `import type * as ns`,
    /// or a per-specifier inline `type` qualifier on every named specifier) to a
    /// listed zone is not reported as a boundary violation. Mixed-specifier
    /// imports (`import { type Foo, Bar }`) that carry at least one value
    /// symbol still fire because the runtime dependency on `Bar` is real.
    /// Type-only re-exports (`export type { Foo } from "..."`) participate
    /// in the same allowance because they surface as edges flagged
    /// `is_type_only: true` and, like type-only imports, are erased at
    /// compile time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_type_only: Vec<String>,
}

/// Resolved boundary config with pre-compiled glob matchers.
#[derive(Debug, Default)]
pub struct ResolvedBoundaryConfig {
    /// Zones with compiled glob matchers for fast file classification.
    pub zones: Vec<ResolvedZone>,
    /// Rules indexed by source zone name.
    pub rules: Vec<ResolvedBoundaryRule>,
}

/// A zone with pre-compiled glob matchers.
#[derive(Debug)]
pub struct ResolvedZone {
    /// Zone identifier.
    pub name: String,
    /// Pre-compiled glob matchers for zone membership.
    /// When `root` is set, matchers are applied to the path with the
    /// `root` prefix stripped (subtree-relative patterns).
    pub matchers: Vec<globset::GlobMatcher>,
    /// Normalized subtree scope (e.g. `"packages/app/"`). When present,
    /// only paths starting with this prefix can match this zone, and the
    /// prefix is stripped before glob matching. Forward slashes only,
    /// always trailing slash. `None` means patterns are matched against
    /// the project-root-relative path as-is.
    pub root: Option<String>,
}

/// A resolved boundary rule.
#[derive(Debug)]
pub struct ResolvedBoundaryRule {
    /// The zone this rule restricts.
    pub from_zone: String,
    /// Zones that `from_zone` is allowed to import from.
    pub allowed_zones: Vec<String>,
    /// Zones that `from_zone` may type-only-import from even when not listed
    /// in `allowed_zones`. See `BoundaryRule::allow_type_only`.
    pub allow_type_only_zones: Vec<String>,
}

impl BoundaryConfig {
    /// Whether any boundaries are configured (including via preset).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.preset.is_none() && self.zones.is_empty()
    }

    /// Expand the preset (if set) into zones and rules, merging user overrides on top.
    ///
    /// `source_root` is the directory prefix for preset zone patterns (e.g., `"src"`).
    /// After expansion, `self.preset` is cleared and all zones/rules are explicit.
    ///
    /// Merge semantics:
    /// - User zones with the same name as a preset zone **replace** the preset zone entirely.
    /// - User rules with the same `from` as a preset rule **replace** the preset rule.
    /// - User zones/rules with new names **add** to the preset set.
    pub fn expand(&mut self, source_root: &str) {
        let Some(preset) = self.preset.take() else {
            return;
        };

        let (preset_zones, preset_rules) = preset.default_config(source_root);

        // Build set of user-defined zone names for override detection.
        let user_zone_names: rustc_hash::FxHashSet<&str> =
            self.zones.iter().map(|z| z.name.as_str()).collect();

        // Start with preset zones, replacing any that the user overrides.
        let mut merged_zones: Vec<BoundaryZone> = preset_zones
            .into_iter()
            .filter(|pz| {
                if user_zone_names.contains(pz.name.as_str()) {
                    tracing::info!(
                        "boundary preset: user zone '{}' replaces preset zone",
                        pz.name
                    );
                    false
                } else {
                    true
                }
            })
            .collect();
        // Append all user zones (both overrides and additions).
        merged_zones.append(&mut self.zones);
        self.zones = merged_zones;

        // Build set of user-defined rule `from` names for override detection.
        let user_rule_sources: rustc_hash::FxHashSet<&str> =
            self.rules.iter().map(|r| r.from.as_str()).collect();

        let mut merged_rules: Vec<BoundaryRule> = preset_rules
            .into_iter()
            .filter(|pr| {
                if user_rule_sources.contains(pr.from.as_str()) {
                    tracing::info!(
                        "boundary preset: user rule for '{}' replaces preset rule",
                        pr.from
                    );
                    false
                } else {
                    true
                }
            })
            .collect();
        merged_rules.append(&mut self.rules);
        self.rules = merged_rules;
    }

    /// Expand auto-discovered boundary groups into concrete child zones.
    ///
    /// A zone with `autoDiscover: ["src/features"]` discovers the immediate
    /// child directories below `src/features` and emits child zones named
    /// `zone_name/child`. Rules that reference the logical parent are expanded
    /// to all discovered children. If the parent also has explicit `patterns`,
    /// it is kept after the children as a fallback so child directories remain
    /// isolated by first-match classification.
    pub fn expand_auto_discover(&mut self, project_root: &Path) {
        if self.zones.iter().all(|zone| zone.auto_discover.is_empty()) {
            return;
        }

        let original_zones = std::mem::take(&mut self.zones);
        let mut expanded_zones = Vec::new();
        let mut group_expansions: rustc_hash::FxHashMap<String, Vec<String>> =
            rustc_hash::FxHashMap::default();

        for mut zone in original_zones {
            if zone.auto_discover.is_empty() {
                expanded_zones.push(zone);
                continue;
            }

            let group_name = zone.name.clone();
            let discovered_zones = discover_child_zones(project_root, &zone);
            let mut expanded_names: Vec<String> = discovered_zones
                .iter()
                .map(|child| child.name.clone())
                .collect();
            expanded_zones.extend(discovered_zones);

            if !zone.patterns.is_empty() {
                // Footgun: top-level files inside the auto-discover directory
                // (e.g. a `src/features/index.ts` barrel) fall back to the
                // parent zone, and the parent rule's allow list typically does
                // not include the discovered child zones, so re-exports from
                // the barrel surface as `parent -> parent/<child>` false
                // positives. The Bulletproof preset deliberately leaves
                // `patterns` empty for this reason.
                if record_auto_discover_patterns_warn_seen(&group_name) {
                    tracing::warn!(
                        "boundary zone '{group_name}' sets BOTH `patterns` and `autoDiscover`. \
                         Top-level files matching the parent pattern fall back to zone '{group_name}' \
                         and may produce false-positive cross-zone violations when they re-export \
                         auto-discovered children (e.g. a `{group_name}/index.ts` barrel). \
                         Drop `patterns` to leave top-level files unclassified, or define explicit \
                         allow rules that include the discovered child zones."
                    );
                }
                expanded_names.push(group_name.clone());
                zone.auto_discover.clear();
                expanded_zones.push(zone);
            }

            if !expanded_names.is_empty() {
                group_expansions
                    .entry(group_name)
                    .or_default()
                    .extend(expanded_names);
            }
        }

        self.zones = expanded_zones;
        if group_expansions.is_empty() {
            return;
        }

        let original_rules = std::mem::take(&mut self.rules);
        let mut generated_rules = Vec::new();
        let mut explicit_rules = Vec::new();
        for rule in original_rules {
            let allow = expand_rule_allow(&rule.allow, &group_expansions);
            let allow_type_only = expand_rule_allow(&rule.allow_type_only, &group_expansions);

            if let Some(from_zones) = group_expansions.get(&rule.from) {
                for from in from_zones {
                    let expanded_rule = BoundaryRule {
                        from: from.clone(),
                        allow: allow.clone(),
                        allow_type_only: allow_type_only.clone(),
                    };
                    if from == &rule.from {
                        explicit_rules.push(expanded_rule);
                    } else {
                        generated_rules.push(expanded_rule);
                    }
                }
            } else {
                explicit_rules.push(BoundaryRule {
                    from: rule.from,
                    allow,
                    allow_type_only,
                });
            }
        }

        let mut expanded_rules = dedupe_rules_keep_last(generated_rules);
        expanded_rules.extend(dedupe_rules_keep_last(explicit_rules));
        self.rules = dedupe_rules_keep_last(expanded_rules);
    }

    /// Return the preset name if one is configured but not yet expanded.
    #[must_use]
    pub fn preset_name(&self) -> Option<&str> {
        self.preset.as_ref().map(|p| match p {
            BoundaryPreset::Layered => "layered",
            BoundaryPreset::Hexagonal => "hexagonal",
            BoundaryPreset::FeatureSliced => "feature-sliced",
            BoundaryPreset::Bulletproof => "bulletproof",
        })
    }

    /// Validate that no zone's pattern redundantly includes its `root`
    /// prefix. Returns a list of error messages tagged with
    /// `FALLOW-BOUNDARY-ROOT-REDUNDANT-PREFIX`. Patterns are resolved
    /// relative to the zone root, so prefixing the pattern with the same
    /// root double-prefixes the path and never matches.
    #[must_use]
    pub fn validate_root_prefixes(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for zone in &self.zones {
            let Some(raw_root) = zone.root.as_deref() else {
                continue;
            };
            let normalized = normalize_zone_root(raw_root);
            // Skip empty-root zones: `""`, `"."`, and `"./"` all normalize to
            // `""`, which behaves as no root at classification time. Without
            // this guard `starts_with("")` is always true and every pattern
            // produces a spurious redundant-prefix error.
            if normalized.is_empty() {
                continue;
            }
            for pattern in &zone.patterns {
                let normalized_pattern = pattern.replace('\\', "/");
                let stripped = normalized_pattern
                    .strip_prefix("./")
                    .unwrap_or(&normalized_pattern);
                if stripped.starts_with(&normalized) {
                    errors.push(format!(
                        "FALLOW-BOUNDARY-ROOT-REDUNDANT-PREFIX: zone '{}': pattern '{}' starts with the zone root '{}'. Patterns are now resolved relative to root; remove the redundant prefix from the pattern.",
                        zone.name, pattern, normalized
                    ));
                }
            }
        }
        errors
    }

    /// Validate that all zone names referenced in rules are defined in `zones`.
    /// Returns a list of (rule_index, undefined_zone_name) pairs.
    ///
    /// Walks every zone-reference surface on `BoundaryRule`: `from`, `allow`,
    /// and `allow_type_only`. An unknown zone in `allow_type_only` silently
    /// behaves as "not allowed" at runtime, so it MUST surface here for parity
    /// with the existing `allow`-side diagnostic.
    #[must_use]
    pub fn validate_zone_references(&self) -> Vec<(usize, &str)> {
        let zone_names: rustc_hash::FxHashSet<&str> =
            self.zones.iter().map(|z| z.name.as_str()).collect();

        let mut errors = Vec::new();
        for (i, rule) in self.rules.iter().enumerate() {
            if !zone_names.contains(rule.from.as_str()) {
                errors.push((i, rule.from.as_str()));
            }
            for allowed in &rule.allow {
                if !zone_names.contains(allowed.as_str()) {
                    errors.push((i, allowed.as_str()));
                }
            }
            for allowed_type_only in &rule.allow_type_only {
                if !zone_names.contains(allowed_type_only.as_str()) {
                    errors.push((i, allowed_type_only.as_str()));
                }
            }
        }
        errors
    }

    /// Resolve into compiled form with pre-built glob matchers.
    /// Invalid glob patterns are logged and skipped.
    #[must_use]
    pub fn resolve(&self) -> ResolvedBoundaryConfig {
        let zones = self
            .zones
            .iter()
            .map(|zone| {
                let matchers = zone
                    .patterns
                    .iter()
                    .filter_map(|pattern| match Glob::new(pattern) {
                        Ok(glob) => Some(glob.compile_matcher()),
                        Err(e) => {
                            tracing::warn!(
                                "invalid boundary zone glob pattern '{}' in zone '{}': {e}",
                                pattern,
                                zone.name
                            );
                            None
                        }
                    })
                    .collect();
                let root = zone.root.as_deref().map(normalize_zone_root);
                ResolvedZone {
                    name: zone.name.clone(),
                    matchers,
                    root,
                }
            })
            .collect();

        let rules = self
            .rules
            .iter()
            .map(|rule| ResolvedBoundaryRule {
                from_zone: rule.from.clone(),
                allowed_zones: rule.allow.clone(),
                allow_type_only_zones: rule.allow_type_only.clone(),
            })
            .collect();

        ResolvedBoundaryConfig { zones, rules }
    }
}

/// Normalize a zone `root` string into the canonical form used at
/// classification time: forward slashes, no leading `./`, always a
/// trailing slash. Empty / `"."` / `"./"` collapse to `""` which means
/// "subtree is the project root" and effectively behaves like no root.
fn normalize_zone_root(raw: &str) -> String {
    let with_slashes = raw.replace('\\', "/");
    let trimmed = with_slashes.trim_start_matches("./");
    let no_dot = if trimmed == "." { "" } else { trimmed };
    if no_dot.is_empty() {
        String::new()
    } else if no_dot.ends_with('/') {
        no_dot.to_owned()
    } else {
        format!("{no_dot}/")
    }
}

fn normalize_auto_discover_dir(raw: &str) -> Option<String> {
    let with_slashes = raw.replace('\\', "/");
    let trimmed = with_slashes.trim_start_matches("./").trim_end_matches('/');
    if trimmed.starts_with('/') || trimmed.split('/').any(|part| part == "..") {
        None
    } else if trimmed == "." {
        Some(String::new())
    } else {
        Some(trimmed.to_owned())
    }
}

fn join_relative_path(prefix: &str, suffix: &str) -> String {
    match (prefix.is_empty(), suffix.is_empty()) {
        (true, true) => String::new(),
        (true, false) => suffix.to_owned(),
        (false, true) => prefix.trim_end_matches('/').to_owned(),
        (false, false) => format!("{}/{}", prefix.trim_end_matches('/'), suffix),
    }
}

fn discover_child_zones(project_root: &Path, zone: &BoundaryZone) -> Vec<BoundaryZone> {
    let mut zones_by_name: rustc_hash::FxHashMap<String, BoundaryZone> =
        rustc_hash::FxHashMap::default();
    let normalized_root = zone
        .root
        .as_deref()
        .map(normalize_zone_root)
        .unwrap_or_default();

    for raw_dir in &zone.auto_discover {
        let Some(discover_dir) = normalize_auto_discover_dir(raw_dir) else {
            tracing::warn!(
                "invalid boundary autoDiscover path '{}' in zone '{}': paths must be project-relative and must not contain '..'",
                raw_dir,
                zone.name
            );
            continue;
        };

        let fs_relative = join_relative_path(&normalized_root, &discover_dir);
        let absolute_dir = if fs_relative.is_empty() {
            project_root.to_path_buf()
        } else {
            project_root.join(&fs_relative)
        };
        let Ok(entries) = std::fs::read_dir(&absolute_dir) else {
            tracing::warn!(
                "boundary zone '{}' autoDiscover path '{}' did not resolve to a readable directory",
                zone.name,
                raw_dir
            );
            continue;
        };

        let mut children: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|file_type| file_type.is_dir()))
            .collect();
        children.sort_by_key(|entry| entry.file_name());

        for child in children {
            let child_name = child.file_name().to_string_lossy().to_string();
            if child_name.is_empty() {
                continue;
            }

            let zone_name = format!("{}/{}", zone.name, child_name);
            let child_pattern = format!("{}/**", join_relative_path(&discover_dir, &child_name));
            let entry = zones_by_name
                .entry(zone_name.clone())
                .or_insert_with(|| BoundaryZone {
                    name: zone_name,
                    patterns: vec![],
                    auto_discover: vec![],
                    root: zone.root.clone(),
                });
            if !entry
                .patterns
                .iter()
                .any(|pattern| pattern == &child_pattern)
            {
                entry.patterns.push(child_pattern);
            }
        }
    }

    let mut zones: Vec<_> = zones_by_name.into_values().collect();
    zones.sort_by(|a, b| a.name.cmp(&b.name));
    zones
}

fn expand_rule_allow(
    allow: &[String],
    group_expansions: &rustc_hash::FxHashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut expanded = Vec::new();
    for zone in allow {
        if let Some(expansion) = group_expansions.get(zone) {
            expanded.extend(expansion.iter().cloned());
        } else {
            expanded.push(zone.clone());
        }
    }
    dedupe_preserving_order(expanded)
}

fn dedupe_preserving_order(values: Vec<String>) -> Vec<String> {
    let mut seen = rustc_hash::FxHashSet::default();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn dedupe_rules_keep_last(rules: Vec<BoundaryRule>) -> Vec<BoundaryRule> {
    let mut seen = rustc_hash::FxHashSet::default();
    let mut deduped: Vec<_> = rules
        .into_iter()
        .rev()
        .filter(|rule| seen.insert(rule.from.clone()))
        .collect();
    deduped.reverse();
    deduped
}

impl ResolvedBoundaryConfig {
    /// Whether any boundaries are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }

    /// Classify a file path into a zone. Returns the first matching zone name.
    /// Path should be relative to the project root with forward slashes.
    ///
    /// When a zone declares a `root` (subtree scope), the path must start
    /// with that prefix and the prefix is stripped before glob matching;
    /// otherwise the zone is skipped. Zones without a `root` keep
    /// project-root-relative behavior.
    #[must_use]
    pub fn classify_zone(&self, relative_path: &str) -> Option<&str> {
        for zone in &self.zones {
            let candidate: &str = match zone.root.as_deref() {
                Some(root) if !root.is_empty() => {
                    let Some(stripped) = relative_path.strip_prefix(root) else {
                        continue;
                    };
                    stripped
                }
                _ => relative_path,
            };
            if zone.matchers.iter().any(|m| m.is_match(candidate)) {
                return Some(&zone.name);
            }
        }
        None
    }

    /// Check if an import from `from_zone` to `to_zone` is allowed.
    /// Returns `true` if the import is permitted.
    #[must_use]
    pub fn is_import_allowed(&self, from_zone: &str, to_zone: &str) -> bool {
        // Self-imports are always allowed.
        if from_zone == to_zone {
            return true;
        }

        // Find the rule for the source zone.
        let rule = self.rules.iter().find(|r| r.from_zone == from_zone);

        match rule {
            // Zone has no rule entry — unrestricted.
            None => true,
            // Zone has a rule — check the allowlist.
            Some(r) => r.allowed_zones.iter().any(|z| z == to_zone),
        }
    }

    /// Check whether a type-only import from `from_zone` to `to_zone` is
    /// permitted by the rule's `allowTypeOnly` list. Only consulted by the
    /// boundary detector after `is_import_allowed` has already returned
    /// `false`; the caller is responsible for verifying the import is in
    /// fact type-only (all symbols on the edge carry the type-only flag).
    /// Returns `false` when no rule exists for `from_zone`, since rule-less
    /// zones are unrestricted and `is_import_allowed` short-circuits before
    /// this is called.
    #[must_use]
    pub fn is_type_only_allowed(&self, from_zone: &str, to_zone: &str) -> bool {
        let Some(rule) = self.rules.iter().find(|r| r.from_zone == from_zone) else {
            return false;
        };
        rule.allow_type_only_zones.iter().any(|z| z == to_zone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config() {
        let config = BoundaryConfig::default();
        assert!(config.is_empty());
        assert!(config.validate_zone_references().is_empty());
    }

    #[test]
    fn deserialize_json() {
        let json = r#"{
            "zones": [
                { "name": "ui", "patterns": ["src/components/**", "src/pages/**"] },
                { "name": "db", "patterns": ["src/db/**"] },
                { "name": "shared", "patterns": ["src/shared/**"] }
            ],
            "rules": [
                { "from": "ui", "allow": ["shared"] },
                { "from": "db", "allow": ["shared"] }
            ]
        }"#;
        let config: BoundaryConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.zones.len(), 3);
        assert_eq!(config.rules.len(), 2);
        assert_eq!(config.zones[0].name, "ui");
        assert_eq!(
            config.zones[0].patterns,
            vec!["src/components/**", "src/pages/**"]
        );
        assert_eq!(config.rules[0].from, "ui");
        assert_eq!(config.rules[0].allow, vec!["shared"]);
    }

    #[test]
    fn deserialize_toml() {
        let toml_str = r#"
[[zones]]
name = "ui"
patterns = ["src/components/**"]

[[zones]]
name = "db"
patterns = ["src/db/**"]

[[rules]]
from = "ui"
allow = ["db"]
"#;
        let config: BoundaryConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.zones.len(), 2);
        assert_eq!(config.rules.len(), 1);
    }

    #[test]
    fn auto_discover_expands_child_zones_and_parent_rules() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("src/features/auth")).unwrap();
        std::fs::create_dir_all(temp.path().join("src/features/billing")).unwrap();

        let mut config = BoundaryConfig {
            preset: None,
            zones: vec![
                BoundaryZone {
                    name: "app".to_string(),
                    patterns: vec!["src/app/**".to_string()],
                    auto_discover: vec![],
                    root: None,
                },
                BoundaryZone {
                    name: "features".to_string(),
                    patterns: vec![],
                    auto_discover: vec!["src/features".to_string()],
                    root: None,
                },
            ],
            rules: vec![
                BoundaryRule {
                    from: "app".to_string(),
                    allow: vec!["features".to_string()],
                    allow_type_only: vec![],
                },
                BoundaryRule {
                    from: "features".to_string(),
                    allow: vec![],
                    allow_type_only: vec![],
                },
            ],
        };

        config.expand_auto_discover(temp.path());

        let zone_names: Vec<_> = config.zones.iter().map(|zone| zone.name.as_str()).collect();
        assert_eq!(zone_names, vec!["app", "features/auth", "features/billing"]);
        assert_eq!(
            config.zones[1].patterns,
            vec!["src/features/auth/**".to_string()]
        );
        assert_eq!(
            config.zones[2].patterns,
            vec!["src/features/billing/**".to_string()]
        );
        let app_rule = config
            .rules
            .iter()
            .find(|rule| rule.from == "app")
            .expect("app rule should be preserved");
        assert_eq!(
            app_rule.allow,
            vec!["features/auth".to_string(), "features/billing".to_string()]
        );
        assert!(
            config
                .rules
                .iter()
                .any(|rule| rule.from == "features/auth" && rule.allow.is_empty())
        );
        assert!(
            config
                .rules
                .iter()
                .any(|rule| rule.from == "features/billing" && rule.allow.is_empty())
        );
        assert!(config.validate_zone_references().is_empty());
    }

    #[test]
    fn auto_discover_explicit_child_rule_wins_over_generated_parent_rule() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("src/features/auth")).unwrap();
        std::fs::create_dir_all(temp.path().join("src/features/billing")).unwrap();

        for explicit_child_first in [true, false] {
            let explicit_child_rule = BoundaryRule {
                from: "features/auth".to_string(),
                allow: vec!["shared".to_string(), "features/billing".to_string()],
                allow_type_only: vec![],
            };
            let parent_rule = BoundaryRule {
                from: "features".to_string(),
                allow: vec!["shared".to_string()],
                allow_type_only: vec![],
            };
            let rules = if explicit_child_first {
                vec![explicit_child_rule, parent_rule]
            } else {
                vec![parent_rule, explicit_child_rule]
            };

            let mut config = BoundaryConfig {
                preset: None,
                zones: vec![
                    BoundaryZone {
                        name: "features".to_string(),
                        patterns: vec![],
                        auto_discover: vec!["src/features".to_string()],
                        root: None,
                    },
                    BoundaryZone {
                        name: "shared".to_string(),
                        patterns: vec!["src/shared/**".to_string()],
                        auto_discover: vec![],
                        root: None,
                    },
                ],
                rules,
            };

            config.expand_auto_discover(temp.path());

            let auth_rule = config
                .rules
                .iter()
                .find(|rule| rule.from == "features/auth")
                .expect("explicit child rule should remain");
            assert_eq!(
                auth_rule.allow,
                vec!["shared".to_string(), "features/billing".to_string()],
                "explicit child rule should win regardless of rule order"
            );

            let billing_rule = config
                .rules
                .iter()
                .find(|rule| rule.from == "features/billing")
                .expect("parent rule should still generate sibling child rule");
            assert_eq!(billing_rule.allow, vec!["shared".to_string()]);
            assert!(config.validate_zone_references().is_empty());
        }
    }

    #[test]
    fn validate_zone_references_valid() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![
                BoundaryZone {
                    name: "ui".to_string(),
                    patterns: vec![],
                    auto_discover: vec![],
                    root: None,
                },
                BoundaryZone {
                    name: "db".to_string(),
                    patterns: vec![],
                    auto_discover: vec![],
                    root: None,
                },
            ],
            rules: vec![BoundaryRule {
                from: "ui".to_string(),
                allow: vec!["db".to_string()],
                allow_type_only: vec![],
            }],
        };
        assert!(config.validate_zone_references().is_empty());
    }

    #[test]
    fn validate_zone_references_invalid_from() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "ui".to_string(),
                patterns: vec![],
                auto_discover: vec![],
                root: None,
            }],
            rules: vec![BoundaryRule {
                from: "nonexistent".to_string(),
                allow: vec!["ui".to_string()],
                allow_type_only: vec![],
            }],
        };
        let errors = config.validate_zone_references();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].1, "nonexistent");
    }

    #[test]
    fn validate_zone_references_invalid_allow() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "ui".to_string(),
                patterns: vec![],
                auto_discover: vec![],
                root: None,
            }],
            rules: vec![BoundaryRule {
                from: "ui".to_string(),
                allow: vec!["nonexistent".to_string()],
                allow_type_only: vec![],
            }],
        };
        let errors = config.validate_zone_references();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].1, "nonexistent");
    }

    #[test]
    fn validate_zone_references_invalid_allow_type_only() {
        // An undefined zone in `allowTypeOnly` silently behaves as "not
        // allowed" at runtime, which the user almost always meant as a typo
        // for an existing zone. Surface the same diagnostic as `allow`.
        let config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "ui".to_string(),
                patterns: vec![],
                auto_discover: vec![],
                root: None,
            }],
            rules: vec![BoundaryRule {
                from: "ui".to_string(),
                allow: vec![],
                allow_type_only: vec!["nonexistent_type_zone".to_string()],
            }],
        };
        let errors = config.validate_zone_references();
        assert_eq!(errors.len(), 1, "got: {errors:?}");
        assert_eq!(errors[0].1, "nonexistent_type_zone");
    }

    #[test]
    fn resolve_and_classify() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![
                BoundaryZone {
                    name: "ui".to_string(),
                    patterns: vec!["src/components/**".to_string()],
                    auto_discover: vec![],
                    root: None,
                },
                BoundaryZone {
                    name: "db".to_string(),
                    patterns: vec!["src/db/**".to_string()],
                    auto_discover: vec![],
                    root: None,
                },
            ],
            rules: vec![],
        };
        let resolved = config.resolve();
        assert_eq!(
            resolved.classify_zone("src/components/Button.tsx"),
            Some("ui")
        );
        assert_eq!(resolved.classify_zone("src/db/queries.ts"), Some("db"));
        assert_eq!(resolved.classify_zone("src/utils/helpers.ts"), None);
    }

    #[test]
    fn first_match_wins() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![
                BoundaryZone {
                    name: "specific".to_string(),
                    patterns: vec!["src/shared/db-utils/**".to_string()],
                    auto_discover: vec![],
                    root: None,
                },
                BoundaryZone {
                    name: "shared".to_string(),
                    patterns: vec!["src/shared/**".to_string()],
                    auto_discover: vec![],
                    root: None,
                },
            ],
            rules: vec![],
        };
        let resolved = config.resolve();
        assert_eq!(
            resolved.classify_zone("src/shared/db-utils/pool.ts"),
            Some("specific")
        );
        assert_eq!(
            resolved.classify_zone("src/shared/helpers.ts"),
            Some("shared")
        );
    }

    #[test]
    fn self_import_always_allowed() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "ui".to_string(),
                patterns: vec![],
                auto_discover: vec![],
                root: None,
            }],
            rules: vec![BoundaryRule {
                from: "ui".to_string(),
                allow: vec![],
                allow_type_only: vec![],
            }],
        };
        let resolved = config.resolve();
        assert!(resolved.is_import_allowed("ui", "ui"));
    }

    #[test]
    fn unrestricted_zone_allows_all() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![
                BoundaryZone {
                    name: "shared".to_string(),
                    patterns: vec![],
                    auto_discover: vec![],
                    root: None,
                },
                BoundaryZone {
                    name: "db".to_string(),
                    patterns: vec![],
                    auto_discover: vec![],
                    root: None,
                },
            ],
            rules: vec![],
        };
        let resolved = config.resolve();
        assert!(resolved.is_import_allowed("shared", "db"));
    }

    #[test]
    fn restricted_zone_blocks_unlisted() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![
                BoundaryZone {
                    name: "ui".to_string(),
                    patterns: vec![],
                    auto_discover: vec![],
                    root: None,
                },
                BoundaryZone {
                    name: "db".to_string(),
                    patterns: vec![],
                    auto_discover: vec![],
                    root: None,
                },
                BoundaryZone {
                    name: "shared".to_string(),
                    patterns: vec![],
                    auto_discover: vec![],
                    root: None,
                },
            ],
            rules: vec![BoundaryRule {
                from: "ui".to_string(),
                allow: vec!["shared".to_string()],
                allow_type_only: vec![],
            }],
        };
        let resolved = config.resolve();
        assert!(resolved.is_import_allowed("ui", "shared"));
        assert!(!resolved.is_import_allowed("ui", "db"));
    }

    #[test]
    fn empty_allow_blocks_all_except_self() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![
                BoundaryZone {
                    name: "isolated".to_string(),
                    patterns: vec![],
                    auto_discover: vec![],
                    root: None,
                },
                BoundaryZone {
                    name: "other".to_string(),
                    patterns: vec![],
                    auto_discover: vec![],
                    root: None,
                },
            ],
            rules: vec![BoundaryRule {
                from: "isolated".to_string(),
                allow: vec![],
                allow_type_only: vec![],
            }],
        };
        let resolved = config.resolve();
        assert!(resolved.is_import_allowed("isolated", "isolated"));
        assert!(!resolved.is_import_allowed("isolated", "other"));
    }

    #[test]
    fn zone_root_filters_classification_to_subtree() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![
                BoundaryZone {
                    name: "ui".to_string(),
                    patterns: vec!["src/**".to_string()],
                    auto_discover: vec![],
                    root: Some("packages/app/".to_string()),
                },
                BoundaryZone {
                    name: "domain".to_string(),
                    patterns: vec!["src/**".to_string()],
                    auto_discover: vec![],
                    root: Some("packages/core/".to_string()),
                },
            ],
            rules: vec![],
        };
        let resolved = config.resolve();
        // Files inside packages/app/ classify as ui
        assert_eq!(
            resolved.classify_zone("packages/app/src/login.tsx"),
            Some("ui")
        );
        // Files inside packages/core/ classify as domain (same pattern, different root)
        assert_eq!(
            resolved.classify_zone("packages/core/src/order.ts"),
            Some("domain")
        );
        // Files outside either subtree do not match
        assert_eq!(resolved.classify_zone("src/login.tsx"), None);
        assert_eq!(resolved.classify_zone("packages/utils/src/x.ts"), None);
    }

    /// Case-sensitivity contract: `root` matching is case-sensitive,
    /// matching the existing globset case-sensitivity for `patterns`. On
    /// case-insensitive filesystems (HFS+, NTFS) two files differing only
    /// in case still classify only when the configured `root` exactly
    /// matches the path's case as fallow recorded it. Locking this down
    /// prevents silent platform-divergent classification.
    #[test]
    fn zone_root_is_case_sensitive() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "ui".to_string(),
                patterns: vec!["src/**".to_string()],
                auto_discover: vec![],
                root: Some("packages/app/".to_string()),
            }],
            rules: vec![],
        };
        let resolved = config.resolve();
        assert_eq!(
            resolved.classify_zone("packages/app/src/login.tsx"),
            Some("ui"),
            "exact-case path classifies"
        );
        assert_eq!(
            resolved.classify_zone("packages/App/src/login.tsx"),
            None,
            "case-different path does not classify (root is case-sensitive)"
        );
        assert_eq!(
            resolved.classify_zone("Packages/app/src/login.tsx"),
            None,
            "case-different prefix does not classify"
        );
    }

    #[test]
    fn zone_root_normalizes_trailing_slash_and_dot_prefix() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![
                BoundaryZone {
                    name: "no-slash".to_string(),
                    patterns: vec!["src/**".to_string()],
                    auto_discover: vec![],
                    root: Some("packages/app".to_string()),
                },
                BoundaryZone {
                    name: "dot-prefixed".to_string(),
                    patterns: vec!["src/**".to_string()],
                    auto_discover: vec![],
                    root: Some("./packages/lib/".to_string()),
                },
            ],
            rules: vec![],
        };
        let resolved = config.resolve();
        assert_eq!(resolved.zones[0].root.as_deref(), Some("packages/app/"));
        assert_eq!(resolved.zones[1].root.as_deref(), Some("packages/lib/"));
        assert_eq!(
            resolved.classify_zone("packages/app/src/x.ts"),
            Some("no-slash")
        );
        assert_eq!(
            resolved.classify_zone("packages/lib/src/x.ts"),
            Some("dot-prefixed")
        );
    }

    #[test]
    fn validate_root_prefixes_flags_redundant_pattern() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "ui".to_string(),
                patterns: vec!["packages/app/src/**".to_string()],
                auto_discover: vec![],
                root: Some("packages/app/".to_string()),
            }],
            rules: vec![],
        };
        let errors = config.validate_root_prefixes();
        assert_eq!(errors.len(), 1, "expected one redundant-prefix error");
        assert!(
            errors[0].contains("FALLOW-BOUNDARY-ROOT-REDUNDANT-PREFIX"),
            "error should be tagged: {}",
            errors[0]
        );
        assert!(
            errors[0].contains("zone 'ui'"),
            "error should name the zone: {}",
            errors[0]
        );
        assert!(
            errors[0].contains("packages/app/src/**"),
            "error should quote the pattern: {}",
            errors[0]
        );
    }

    #[test]
    fn validate_root_prefixes_handles_unnormalized_root() {
        // Root without trailing slash + pattern with leading "./" should
        // still be detected as redundant after normalization.
        let config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "ui".to_string(),
                patterns: vec!["./packages/app/src/**".to_string()],
                auto_discover: vec![],
                root: Some("packages/app".to_string()),
            }],
            rules: vec![],
        };
        let errors = config.validate_root_prefixes();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn validate_root_prefixes_empty_when_no_overlap() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "ui".to_string(),
                patterns: vec!["src/**".to_string()],
                auto_discover: vec![],
                root: Some("packages/app/".to_string()),
            }],
            rules: vec![],
        };
        assert!(config.validate_root_prefixes().is_empty());
    }

    #[test]
    fn validate_root_prefixes_skips_zones_without_root() {
        let json = r#"{
            "zones": [{ "name": "ui", "patterns": ["src/**"] }],
            "rules": []
        }"#;
        let config: BoundaryConfig = serde_json::from_str(json).unwrap();
        assert!(config.validate_root_prefixes().is_empty());
    }

    /// Regression: an empty `root` (or `"."`/`"./"`, both of which normalize
    /// to `""`) used to make `starts_with("")` always true, producing a
    /// spurious FALLOW-BOUNDARY-ROOT-REDUNDANT-PREFIX error for every
    /// pattern in the zone. The validation must skip empty-normalized roots
    /// the same way `classify_zone` does.
    #[test]
    fn validate_root_prefixes_skips_empty_root() {
        for raw_root in ["", ".", "./"] {
            let config = BoundaryConfig {
                preset: None,
                zones: vec![BoundaryZone {
                    name: "ui".to_string(),
                    patterns: vec!["src/**".to_string(), "lib/**".to_string()],
                    auto_discover: vec![],
                    root: Some(raw_root.to_string()),
                }],
                rules: vec![],
            };
            let errors = config.validate_root_prefixes();
            assert!(
                errors.is_empty(),
                "empty-normalized root {raw_root:?} produced spurious errors: {errors:?}"
            );
        }
    }

    #[test]
    fn deserialize_zone_with_root() {
        let json = r#"{
            "zones": [
                { "name": "ui", "patterns": ["src/**"], "root": "packages/app/" }
            ],
            "rules": []
        }"#;
        let config: BoundaryConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.zones[0].root.as_deref(), Some("packages/app/"));
    }

    // ── Preset deserialization ─────────────────────────────────

    #[test]
    fn deserialize_preset_json() {
        let json = r#"{ "preset": "layered" }"#;
        let config: BoundaryConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.preset, Some(BoundaryPreset::Layered));
        assert!(config.zones.is_empty());
    }

    #[test]
    fn deserialize_preset_hexagonal_json() {
        let json = r#"{ "preset": "hexagonal" }"#;
        let config: BoundaryConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.preset, Some(BoundaryPreset::Hexagonal));
    }

    #[test]
    fn deserialize_preset_feature_sliced_json() {
        let json = r#"{ "preset": "feature-sliced" }"#;
        let config: BoundaryConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.preset, Some(BoundaryPreset::FeatureSliced));
    }

    #[test]
    fn deserialize_preset_toml() {
        let toml_str = r#"preset = "layered""#;
        let config: BoundaryConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.preset, Some(BoundaryPreset::Layered));
    }

    #[test]
    fn deserialize_invalid_preset_rejected() {
        let json = r#"{ "preset": "invalid_preset" }"#;
        let result: Result<BoundaryConfig, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn preset_absent_by_default() {
        let config = BoundaryConfig::default();
        assert!(config.preset.is_none());
        assert!(config.is_empty());
    }

    #[test]
    fn preset_makes_config_non_empty() {
        let config = BoundaryConfig {
            preset: Some(BoundaryPreset::Layered),
            zones: vec![],
            rules: vec![],
        };
        assert!(!config.is_empty());
    }

    // ── Preset expansion ───────────────────────────────────────

    #[test]
    fn expand_layered_produces_four_zones() {
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::Layered),
            zones: vec![],
            rules: vec![],
        };
        config.expand("src");
        assert_eq!(config.zones.len(), 4);
        assert_eq!(config.rules.len(), 4);
        assert!(config.preset.is_none(), "preset cleared after expand");
        assert_eq!(config.zones[0].name, "presentation");
        assert_eq!(config.zones[0].patterns, vec!["src/presentation/**"]);
    }

    #[test]
    fn expand_layered_rules_correct() {
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::Layered),
            zones: vec![],
            rules: vec![],
        };
        config.expand("src");
        // presentation → application only
        let pres_rule = config
            .rules
            .iter()
            .find(|r| r.from == "presentation")
            .unwrap();
        assert_eq!(pres_rule.allow, vec!["application"]);
        // application → domain only
        let app_rule = config
            .rules
            .iter()
            .find(|r| r.from == "application")
            .unwrap();
        assert_eq!(app_rule.allow, vec!["domain"]);
        // domain → nothing
        let dom_rule = config.rules.iter().find(|r| r.from == "domain").unwrap();
        assert!(dom_rule.allow.is_empty());
        // infrastructure → domain + application (DI-friendly)
        let infra_rule = config
            .rules
            .iter()
            .find(|r| r.from == "infrastructure")
            .unwrap();
        assert_eq!(infra_rule.allow, vec!["domain", "application"]);
    }

    #[test]
    fn expand_hexagonal_produces_three_zones() {
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::Hexagonal),
            zones: vec![],
            rules: vec![],
        };
        config.expand("src");
        assert_eq!(config.zones.len(), 3);
        assert_eq!(config.rules.len(), 3);
        assert_eq!(config.zones[0].name, "adapters");
        assert_eq!(config.zones[1].name, "ports");
        assert_eq!(config.zones[2].name, "domain");
    }

    #[test]
    fn expand_feature_sliced_produces_six_zones() {
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::FeatureSliced),
            zones: vec![],
            rules: vec![],
        };
        config.expand("src");
        assert_eq!(config.zones.len(), 6);
        assert_eq!(config.rules.len(), 6);
        // app can import everything below
        let app_rule = config.rules.iter().find(|r| r.from == "app").unwrap();
        assert_eq!(
            app_rule.allow,
            vec!["pages", "widgets", "features", "entities", "shared"]
        );
        // shared imports nothing
        let shared_rule = config.rules.iter().find(|r| r.from == "shared").unwrap();
        assert!(shared_rule.allow.is_empty());
        // entities → shared only
        let ent_rule = config.rules.iter().find(|r| r.from == "entities").unwrap();
        assert_eq!(ent_rule.allow, vec!["shared"]);
    }

    #[test]
    fn expand_bulletproof_produces_four_zones() {
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::Bulletproof),
            zones: vec![],
            rules: vec![],
        };
        config.expand("src");
        assert_eq!(config.zones.len(), 4);
        assert_eq!(config.rules.len(), 4);
        assert_eq!(config.zones[0].name, "app");
        assert_eq!(config.zones[1].name, "features");
        assert_eq!(config.zones[2].name, "shared");
        assert_eq!(config.zones[3].name, "server");
        // shared zone has multiple patterns
        assert!(config.zones[2].patterns.len() > 1);
        assert!(
            config.zones[2]
                .patterns
                .contains(&"src/components/**".to_string())
        );
        assert!(
            config.zones[2]
                .patterns
                .contains(&"src/hooks/**".to_string())
        );
        assert!(config.zones[2].patterns.contains(&"src/lib/**".to_string()));
        assert!(
            config.zones[2]
                .patterns
                .contains(&"src/providers/**".to_string())
        );
    }

    #[test]
    fn expand_bulletproof_rules_correct() {
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::Bulletproof),
            zones: vec![],
            rules: vec![],
        };
        config.expand("src");
        // app → features, shared, server
        let app_rule = config.rules.iter().find(|r| r.from == "app").unwrap();
        assert_eq!(app_rule.allow, vec!["features", "shared", "server"]);
        // features → shared, server
        let feat_rule = config.rules.iter().find(|r| r.from == "features").unwrap();
        assert_eq!(feat_rule.allow, vec!["shared", "server"]);
        // server → shared
        let srv_rule = config.rules.iter().find(|r| r.from == "server").unwrap();
        assert_eq!(srv_rule.allow, vec!["shared"]);
        // shared → nothing (isolated)
        let shared_rule = config.rules.iter().find(|r| r.from == "shared").unwrap();
        assert!(shared_rule.allow.is_empty());
    }

    #[test]
    fn expand_bulletproof_then_resolve_classifies() {
        // `expand()` alone (without `expand_auto_discover`) does not produce
        // the per-feature child zones, so the `features` group is empty and
        // top-level `src/features/...` files are unclassified. Sibling
        // `app` / `shared` / `server` zones still classify normally.
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::Bulletproof),
            zones: vec![],
            rules: vec![],
        };
        config.expand("src");
        let resolved = config.resolve();
        assert_eq!(
            resolved.classify_zone("src/app/dashboard/page.tsx"),
            Some("app")
        );
        assert_eq!(
            resolved.classify_zone("src/features/auth/hooks/useAuth.ts"),
            None,
            "without expand_auto_discover, src/features/... is unclassified"
        );
        assert_eq!(
            resolved.classify_zone("src/components/Button/Button.tsx"),
            Some("shared")
        );
        assert_eq!(
            resolved.classify_zone("src/hooks/useFormatters.ts"),
            Some("shared")
        );
        assert_eq!(
            resolved.classify_zone("src/server/db/schema/users.ts"),
            Some("server")
        );
        // features cannot import shared directly — only via allowed rules
        assert!(resolved.is_import_allowed("features", "shared"));
        assert!(resolved.is_import_allowed("features", "server"));
        assert!(!resolved.is_import_allowed("features", "app"));
        assert!(!resolved.is_import_allowed("shared", "features"));
        assert!(!resolved.is_import_allowed("server", "features"));
    }

    /// Regression for the bulletproof barrel pattern: a top-level
    /// `src/features/index.ts` barrel re-exporting child features must NOT
    /// trigger `features → features/<child>` boundary violations. The fix is
    /// to keep the bulletproof `features` zone pattern-free so the barrel is
    /// unclassified (unrestricted) while child zones still enforce sibling
    /// isolation.
    #[test]
    fn bulletproof_features_barrel_is_unclassified() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("src/features/auth")).unwrap();
        std::fs::create_dir_all(temp.path().join("src/features/billing")).unwrap();

        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::Bulletproof),
            zones: vec![],
            rules: vec![],
        };
        config.expand("src");
        config.expand_auto_discover(temp.path());
        let resolved = config.resolve();

        // Top-level barrel inside src/features stays unclassified.
        assert_eq!(
            resolved.classify_zone("src/features/index.ts"),
            None,
            "src/features/index.ts barrel must be unclassified to allow re-exporting children"
        );
        // Discovered child zones still classify normally.
        assert_eq!(
            resolved.classify_zone("src/features/auth/login.ts"),
            Some("features/auth")
        );
        assert_eq!(
            resolved.classify_zone("src/features/billing/invoice.ts"),
            Some("features/billing")
        );
        // Sibling-feature import is still a cross-zone violation.
        assert!(!resolved.is_import_allowed("features/auth", "features/billing"));
    }

    #[test]
    fn expand_uses_custom_source_root() {
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::Hexagonal),
            zones: vec![],
            rules: vec![],
        };
        config.expand("lib");
        assert_eq!(config.zones[0].patterns, vec!["lib/adapters/**"]);
        assert_eq!(config.zones[2].patterns, vec!["lib/domain/**"]);
    }

    // ── Preset merge behavior ──────────────────────────────────

    #[test]
    fn user_zone_replaces_preset_zone() {
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::Hexagonal),
            zones: vec![BoundaryZone {
                name: "domain".to_string(),
                patterns: vec!["src/core/**".to_string()],
                auto_discover: vec![],
                root: None,
            }],
            rules: vec![],
        };
        config.expand("src");
        // 3 zones total: adapters + ports from preset, domain from user
        assert_eq!(config.zones.len(), 3);
        let domain = config.zones.iter().find(|z| z.name == "domain").unwrap();
        assert_eq!(domain.patterns, vec!["src/core/**"]);
    }

    #[test]
    fn user_zone_adds_to_preset() {
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::Hexagonal),
            zones: vec![BoundaryZone {
                name: "shared".to_string(),
                patterns: vec!["src/shared/**".to_string()],
                auto_discover: vec![],
                root: None,
            }],
            rules: vec![],
        };
        config.expand("src");
        assert_eq!(config.zones.len(), 4); // 3 preset + 1 user
        assert!(config.zones.iter().any(|z| z.name == "shared"));
    }

    #[test]
    fn user_rule_replaces_preset_rule() {
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::Hexagonal),
            zones: vec![],
            rules: vec![BoundaryRule {
                from: "adapters".to_string(),
                allow: vec!["ports".to_string(), "domain".to_string()],
                allow_type_only: vec![],
            }],
        };
        config.expand("src");
        let adapter_rule = config.rules.iter().find(|r| r.from == "adapters").unwrap();
        // User rule allows both ports and domain (preset only allowed ports)
        assert_eq!(adapter_rule.allow, vec!["ports", "domain"]);
        // Other preset rules untouched
        assert_eq!(
            config.rules.iter().filter(|r| r.from == "adapters").count(),
            1
        );
    }

    #[test]
    fn expand_without_preset_is_noop() {
        let mut config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "ui".to_string(),
                patterns: vec!["src/ui/**".to_string()],
                auto_discover: vec![],
                root: None,
            }],
            rules: vec![],
        };
        config.expand("src");
        assert_eq!(config.zones.len(), 1);
        assert_eq!(config.zones[0].name, "ui");
    }

    #[test]
    fn expand_then_validate_succeeds() {
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::Layered),
            zones: vec![],
            rules: vec![],
        };
        config.expand("src");
        assert!(config.validate_zone_references().is_empty());
    }

    #[test]
    fn expand_then_resolve_classifies() {
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::Hexagonal),
            zones: vec![],
            rules: vec![],
        };
        config.expand("src");
        let resolved = config.resolve();
        assert_eq!(
            resolved.classify_zone("src/adapters/http/handler.ts"),
            Some("adapters")
        );
        assert_eq!(resolved.classify_zone("src/domain/user.ts"), Some("domain"));
        assert!(!resolved.is_import_allowed("adapters", "domain"));
        assert!(resolved.is_import_allowed("adapters", "ports"));
    }

    #[test]
    fn preset_name_returns_correct_string() {
        let config = BoundaryConfig {
            preset: Some(BoundaryPreset::FeatureSliced),
            zones: vec![],
            rules: vec![],
        };
        assert_eq!(config.preset_name(), Some("feature-sliced"));

        let empty = BoundaryConfig::default();
        assert_eq!(empty.preset_name(), None);
    }

    #[test]
    fn preset_name_all_variants() {
        let cases = [
            (BoundaryPreset::Layered, "layered"),
            (BoundaryPreset::Hexagonal, "hexagonal"),
            (BoundaryPreset::FeatureSliced, "feature-sliced"),
            (BoundaryPreset::Bulletproof, "bulletproof"),
        ];
        for (preset, expected_name) in cases {
            let config = BoundaryConfig {
                preset: Some(preset),
                zones: vec![],
                rules: vec![],
            };
            assert_eq!(
                config.preset_name(),
                Some(expected_name),
                "preset_name() mismatch for variant"
            );
        }
    }

    // ── ResolvedBoundaryConfig::is_empty ────────────────────────────

    #[test]
    fn resolved_boundary_config_empty() {
        let resolved = ResolvedBoundaryConfig::default();
        assert!(resolved.is_empty());
    }

    #[test]
    fn resolved_boundary_config_with_zones_not_empty() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "ui".to_string(),
                patterns: vec!["src/ui/**".to_string()],
                auto_discover: vec![],
                root: None,
            }],
            rules: vec![],
        };
        let resolved = config.resolve();
        assert!(!resolved.is_empty());
    }

    // ── BoundaryConfig::is_empty edge cases ─────────────────────────

    #[test]
    fn boundary_config_with_only_rules_is_empty() {
        // Having rules but no zones/preset is still "empty" since rules without zones
        // cannot produce boundary violations.
        let config = BoundaryConfig {
            preset: None,
            zones: vec![],
            rules: vec![BoundaryRule {
                from: "ui".to_string(),
                allow: vec!["db".to_string()],
                allow_type_only: vec![],
            }],
        };
        assert!(config.is_empty());
    }

    #[test]
    fn boundary_config_with_zones_not_empty() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "ui".to_string(),
                patterns: vec![],
                auto_discover: vec![],
                root: None,
            }],
            rules: vec![],
        };
        assert!(!config.is_empty());
    }

    // ── Multiple zone patterns ──────────────────────────────────────

    #[test]
    fn zone_with_multiple_patterns_matches_any() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "ui".to_string(),
                patterns: vec![
                    "src/components/**".to_string(),
                    "src/pages/**".to_string(),
                    "src/views/**".to_string(),
                ],
                auto_discover: vec![],
                root: None,
            }],
            rules: vec![],
        };
        let resolved = config.resolve();
        assert_eq!(
            resolved.classify_zone("src/components/Button.tsx"),
            Some("ui")
        );
        assert_eq!(resolved.classify_zone("src/pages/Home.tsx"), Some("ui"));
        assert_eq!(
            resolved.classify_zone("src/views/Dashboard.tsx"),
            Some("ui")
        );
        assert_eq!(resolved.classify_zone("src/utils/helpers.ts"), None);
    }

    // ── validate_zone_references with multiple errors ───────────────

    #[test]
    fn validate_zone_references_multiple_errors() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "ui".to_string(),
                patterns: vec![],
                auto_discover: vec![],
                root: None,
            }],
            rules: vec![
                BoundaryRule {
                    from: "nonexistent_from".to_string(),
                    allow: vec!["nonexistent_allow".to_string()],
                    allow_type_only: vec![],
                },
                BoundaryRule {
                    from: "ui".to_string(),
                    allow: vec!["also_nonexistent".to_string()],
                    allow_type_only: vec![],
                },
            ],
        };
        let errors = config.validate_zone_references();
        // Rule 0: invalid "from" + invalid "allow" = 2 errors
        // Rule 1: valid "from", invalid "allow" = 1 error
        assert_eq!(errors.len(), 3);
    }

    // ── Preset expansion with custom source root ────────────────────

    #[test]
    fn expand_feature_sliced_with_custom_root() {
        let mut config = BoundaryConfig {
            preset: Some(BoundaryPreset::FeatureSliced),
            zones: vec![],
            rules: vec![],
        };
        config.expand("lib");
        assert_eq!(config.zones[0].patterns, vec!["lib/app/**"]);
        assert_eq!(config.zones[5].patterns, vec!["lib/shared/**"]);
    }

    // ── is_import_allowed for zone not in rules (unrestricted) ──────

    #[test]
    fn zone_not_in_rules_is_unrestricted() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![
                BoundaryZone {
                    name: "a".to_string(),
                    patterns: vec![],
                    auto_discover: vec![],
                    root: None,
                },
                BoundaryZone {
                    name: "b".to_string(),
                    patterns: vec![],
                    auto_discover: vec![],
                    root: None,
                },
                BoundaryZone {
                    name: "c".to_string(),
                    patterns: vec![],
                    auto_discover: vec![],
                    root: None,
                },
            ],
            rules: vec![BoundaryRule {
                from: "a".to_string(),
                allow: vec!["b".to_string()],
                allow_type_only: vec![],
            }],
        };
        let resolved = config.resolve();
        // "a" is restricted: can import from "b" but not "c"
        assert!(resolved.is_import_allowed("a", "b"));
        assert!(!resolved.is_import_allowed("a", "c"));
        // "b" has no rule entry: unrestricted
        assert!(resolved.is_import_allowed("b", "a"));
        assert!(resolved.is_import_allowed("b", "c"));
        // "c" has no rule entry: unrestricted
        assert!(resolved.is_import_allowed("c", "a"));
    }

    // ── Preset serialization/deserialization roundtrip ───────────────

    #[test]
    fn boundary_preset_json_roundtrip() {
        let presets = [
            BoundaryPreset::Layered,
            BoundaryPreset::Hexagonal,
            BoundaryPreset::FeatureSliced,
            BoundaryPreset::Bulletproof,
        ];
        for preset in presets {
            let json = serde_json::to_string(&preset).unwrap();
            let restored: BoundaryPreset = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, preset);
        }
    }

    #[test]
    fn deserialize_preset_bulletproof_json() {
        let json = r#"{ "preset": "bulletproof" }"#;
        let config: BoundaryConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.preset, Some(BoundaryPreset::Bulletproof));
    }

    // ── Zone with invalid glob ──────────────────────────────────────

    #[test]
    fn resolve_skips_invalid_zone_glob() {
        let config = BoundaryConfig {
            preset: None,
            zones: vec![BoundaryZone {
                name: "broken".to_string(),
                patterns: vec!["[invalid".to_string()],
                auto_discover: vec![],
                root: None,
            }],
            rules: vec![],
        };
        let resolved = config.resolve();
        // Zone exists but has no valid matchers, so no file can be classified into it
        assert!(!resolved.is_empty());
        assert_eq!(resolved.classify_zone("anything.ts"), None);
    }
}
