//! `fallow audit --brief` (alias `fallow review`): a deterministic rendering
//! mode layered over the existing audit analysis.
//!
//! The brief answers "where do I look?" rather than "will CI block this?". It
//! is composition + rendering over the same [`crate::audit::AuditResult`] that
//! drives `fallow audit`; it runs no new analysis and, critically, ALWAYS exits
//! 0 so a reviewer or agent can read the orientation even when the underlying
//! audit verdict is `fail`. The verdict is still computed and carried in the
//! brief JSON informationally, but it never drives the exit code on this path.
//!
//! The JSON envelope is an independently-versioned
//! [`crate::output_envelope::FallowOutput::AuditBrief`] variant
//! (`kind: "audit-brief"`) so the brief shape evolves on its own cadence
//! without bumping the main `--format json` contract.

use std::process::ExitCode;

use fallow_types::results::AnalysisResults;
use rustc_hash::FxHashSet;
use serde::Serialize;

use crate::audit::AuditResult;

/// Wire version for the `fallow audit --brief --format json` envelope. Bumped
/// independently of the main `--format json` contract: the brief shape can grow
/// (populating `hunks`, `net_lines`, `exports_added`, `reachable_from`) without
/// touching `report::SCHEMA_VERSION`.
pub const REVIEW_BRIEF_SCHEMA_VERSION: u32 = 1;

/// A file count at or above which a changeset is classified [`RiskClass::High`].
const RISK_HIGH_FILES: usize = 20;
/// A net-line count at or above which a changeset is classified
/// [`RiskClass::High`]. Stub-only in v1 (net lines are not yet threaded); kept
/// as a named constant so the threshold is documented where the classifier
/// lives.
const RISK_HIGH_LINES: i64 = 500;
/// A file count at or above which a changeset is classified
/// [`RiskClass::Medium`].
const RISK_MEDIUM_FILES: usize = 5;
/// A net-line count at or above which a changeset is classified
/// [`RiskClass::Medium`].
const RISK_MEDIUM_LINES: i64 = 100;

/// Independently-versioned wire-version newtype for the brief envelope.
/// Serializes as the integer `REVIEW_BRIEF_SCHEMA_VERSION`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReviewBriefSchemaVersion(pub u32);

impl Default for ReviewBriefSchemaVersion {
    fn default() -> Self {
        Self(REVIEW_BRIEF_SCHEMA_VERSION)
    }
}

/// Coarse risk classification for a changeset, a pure function of the change
/// size (file count plus, once threaded, net lines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RiskClass {
    /// Small, contained change.
    Low,
    /// Moderately sized change.
    Medium,
    /// Large change spanning many files or lines.
    High,
}

/// Suggested reviewer effort, a pure function of [`RiskClass`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReviewEffort {
    /// A quick scan is enough.
    Glance,
    /// A normal line-by-line review.
    Review,
    /// A careful, deep review is warranted.
    DeepDive,
}

/// Stage 0 of the brief: triage facts derived purely from the diff size.
///
/// `hunks` and `net_lines` are `None` in v1: the file-level audit does not yet
/// thread a `DiffIndex` (from `report/ci/diff_filter.rs`). They populate later,
/// on `--diff-file` / `--diff-stdin`, without a schema bump.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct DiffTriage {
    /// Number of changed files in the audit scope.
    pub files: usize,
    /// Number of diff hunks. `None` in v1 (no diff index threaded yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hunks: Option<usize>,
    /// Net added-minus-removed lines. `None` in v1 (no diff index threaded yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net_lines: Option<i64>,
    /// Coarse risk class derived from the change size.
    pub risk_class: RiskClass,
    /// Suggested reviewer effort derived from `risk_class`.
    pub review_effort: ReviewEffort,
}

/// Stage 1 of the brief: graph-derived orientation facts.
///
/// `boundaries_touched` is derived from the run's boundary-violation zones;
/// `reachable_from` is populated by the E2 impact closure (the affected-not-shown
/// set: modules the changed code is reachable from / affects, none in the diff).
/// `exports_added` / `api_width_delta` stay honestly stubbed (`0`) until the
/// export-surface delta lands (E3). The fields are present and correctly typed so
/// values fill in later without a schema bump.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GraphFacts {
    /// Number of exports added by the changeset. Stubbed to `0` in v1.
    pub exports_added: usize,
    /// Change in public API width (added minus removed exports). Stubbed to `0`
    /// in v1.
    pub api_width_delta: i64,
    /// Root-relative paths of modules the changed code is reachable from / affects
    /// (the impact closure's affected-but-not-in-diff set), deduped and sorted.
    /// Empty when no graph was retained or nothing depends on the changed files.
    pub reachable_from: Vec<String>,
    /// Architecture boundary zones touched by the changeset, deduped and sorted.
    /// Derived from the run's boundary-violation findings.
    pub boundaries_touched: Vec<String>,
}

/// Stage 3 of the brief: the impact closure (E2). The transitive
/// affected-but-not-in-diff set plus the coordination gap. The differentiator a
/// diff tool fundamentally cannot do, because it has no graph.
///
/// Honest scope (ADR-001, syntactic): the coordination gap is an attention
/// pointer at the exact inter-module failure mode, NOT a correctness proof.
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ImpactClosureFacts {
    /// Root-relative paths transitively affected by the changeset (reverse-deps +
    /// re-export chains) that are NOT in the diff, deduped and sorted.
    pub affected_not_shown: Vec<String>,
    /// Coordination gaps: a changed file exports a contract consumed by a module
    /// absent from the diff. One entry per (changed file, consumer) pair.
    pub coordination_gap: Vec<CoordinationGapFact>,
}

/// One coordination-gap entry: a changed file exports symbols consumed by a
/// `consumer_file` that is NOT in the diff. Deduped per (changed, consumer) pair
/// (firing-precision rule R2).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct CoordinationGapFact {
    /// Root-relative path of the changed file whose contract is consumed elsewhere.
    pub changed_file: String,
    /// Root-relative path of the consumer module that is NOT in the diff.
    pub consumer_file: String,
    /// The exported symbol names the consumer references, sorted.
    pub consumed_symbols: Vec<String>,
    /// Honest scope note: this is a syntactic attention pointer, not a proof.
    pub note: String,
}

/// The honest-scope note stamped on every coordination-gap entry (ADR-001).
const COORDINATION_GAP_NOTE: &str = "syntactic attention pointer, not a correctness proof";

/// E3 diff-aware deterministic deltas (6.A), framed new-vs-pre-existing against
/// the audit base snapshot. Each entry is a brief summary/verdict line.
///
/// `public_api` is batch-consolidated to ONE decision per change (E0 rule R1):
/// the `added` list carries the introduced public-export keys as evidence, but a
/// reviewer reads "the public surface widened by N", never one decision per
/// symbol.
#[derive(Debug, Clone, Default, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ReviewDeltas {
    /// Cross-zone boundary EDGES introduced vs base (R2 first-edge-only: one per
    /// `<from_zone>-><to_zone>` pair, never per import). New-vs-pre-existing.
    pub boundary_introduced: Vec<String>,
    /// Circular dependencies introduced vs base (canonical file-set keys).
    pub cycle_introduced: Vec<String>,
    /// Exports-aware public-API surface delta: the public-export keys
    /// (`<rel_path>::<name>`) added vs base, resolved through `package.json`
    /// `exports` + re-export reachability. A symbol re-exported only through an
    /// internal barrel NOT in `exports` is absent here (zero delta); one
    /// reachable through an `exports` path is present (exactly one).
    pub public_api_added: Vec<String>,
}

/// Build the E3 deltas from head sets vs a base set, sorted for determinism.
#[must_use]
#[allow(
    clippy::implicit_hasher,
    reason = "callers always pass the audit FxHashSet key sets; generalizing the hasher adds noise"
)]
pub fn build_review_deltas(
    head_boundary: &FxHashSet<String>,
    base_boundary: &FxHashSet<String>,
    head_cycles: &FxHashSet<String>,
    base_cycles: &FxHashSet<String>,
    head_public_api: &FxHashSet<String>,
    base_public_api: &FxHashSet<String>,
) -> ReviewDeltas {
    use crate::audit::review_deltas::introduced_keys;
    ReviewDeltas {
        boundary_introduced: introduced_keys(head_boundary, base_boundary),
        cycle_introduced: introduced_keys(head_cycles, base_cycles),
        public_api_added: introduced_keys(head_public_api, base_public_api),
    }
}

/// The full `fallow audit --brief --format json` envelope. Carries the
/// informational verdict, the triage and graph-facts orientation stages, plus
/// the reused "subtract" section (the same dead-code / duplication / complexity
/// payload `fallow audit --format json` emits).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(title = "fallow audit --brief --format json")
)]
pub struct ReviewBriefOutput {
    /// Independently-versioned brief schema version.
    pub schema_version: ReviewBriefSchemaVersion,
    /// Fallow CLI version that produced this output.
    pub version: String,
    /// Command discriminator singleton: always `"audit-brief"`.
    pub command: String,
    /// Stage 0: diff triage.
    pub triage: DiffTriage,
    /// Stage 1: graph orientation facts.
    pub graph_facts: GraphFacts,
    /// Stage 3: the impact closure (affected-not-shown + coordination gap).
    pub impact_closure: ImpactClosureFacts,
    /// E3 (6.A): diff-aware deterministic deltas (boundary/cycle introduced +
    /// exports-aware public-API surface delta), new-vs-pre-existing.
    pub deltas: ReviewDeltas,
    /// E3 (6.F, headline): reviewer-private weakening signals (tests
    /// removed/skipped, thresholds lowered, suppressions added, security steps
    /// removed). Advisory, never gates, never auto-posted.
    pub weakening: Vec<crate::audit::weakening::WeakeningSignal>,
    /// E3 (6.D): ownership-aware reviewer routing (per-file expert + bus-factor).
    pub routing: crate::audit::routing::RoutingFacts,
}

/// Classify a changeset's risk purely from its size. `net_lines` is consulted
/// when present (it is `None` on the v1 file-level audit path).
#[must_use]
pub fn classify_risk(files: usize, net_lines: Option<i64>) -> RiskClass {
    let lines = net_lines.unwrap_or(0).abs();
    if files >= RISK_HIGH_FILES || lines >= RISK_HIGH_LINES {
        RiskClass::High
    } else if files >= RISK_MEDIUM_FILES || lines >= RISK_MEDIUM_LINES {
        RiskClass::Medium
    } else {
        RiskClass::Low
    }
}

/// Map a [`RiskClass`] to the suggested reviewer effort.
#[must_use]
pub fn review_effort_for(risk: RiskClass) -> ReviewEffort {
    match risk {
        RiskClass::Low => ReviewEffort::Glance,
        RiskClass::Medium => ReviewEffort::Review,
        RiskClass::High => ReviewEffort::DeepDive,
    }
}

/// Build the Stage 0 triage facts from the audit result.
#[must_use]
pub fn build_triage(result: &AuditResult) -> DiffTriage {
    let files = result.changed_files_count;
    // v1: no diff index is threaded into the file-level audit, so hunks and net
    // lines are honestly absent. They populate on `--diff-file`/`--diff-stdin`.
    let hunks = None;
    let net_lines = None;
    let risk_class = classify_risk(files, net_lines);
    DiffTriage {
        files,
        hunks,
        net_lines,
        risk_class,
        review_effort: review_effort_for(risk_class),
    }
}

/// Derive the Stage 1 graph facts from the analysis results plus the E2 impact
/// closure.
///
/// `boundaries_touched` is the deduped, sorted boundary-violation zone set;
/// `reachable_from` is the impact closure's affected-not-shown set (modules the
/// changed code reaches / affects, none in the diff). `exports_added` /
/// `api_width_delta` stay stubbed until the export-surface delta (E3).
#[must_use]
pub fn derive_graph_facts(
    results: &AnalysisResults,
    closure: Option<&fallow_core::graph::ImpactClosurePaths>,
) -> GraphFacts {
    let mut zones: FxHashSet<String> = FxHashSet::default();
    for finding in &results.boundary_violations {
        zones.insert(finding.violation.from_zone.clone());
        zones.insert(finding.violation.to_zone.clone());
    }
    let mut boundaries_touched: Vec<String> = zones.into_iter().collect();
    boundaries_touched.sort();

    let reachable_from = closure
        .map(|c| c.affected_not_shown.clone())
        .unwrap_or_default();

    GraphFacts {
        exports_added: 0,
        api_width_delta: 0,
        reachable_from,
        boundaries_touched,
    }
}

/// Build the Stage 3 impact-closure facts from the audit result's retained
/// closure (computed on the brief path). Returns an empty closure when no graph
/// was retained (the closure is `None`).
#[must_use]
fn build_impact_closure_facts(result: &AuditResult) -> ImpactClosureFacts {
    let Some(closure) = result
        .check
        .as_ref()
        .and_then(|c| c.impact_closure.as_ref())
    else {
        return ImpactClosureFacts::default();
    };
    let coordination_gap = closure
        .coordination_gap
        .iter()
        .map(|gap| CoordinationGapFact {
            changed_file: gap.changed_file.clone(),
            consumer_file: gap.consumer_file.clone(),
            consumed_symbols: gap.consumed_symbols.clone(),
            note: COORDINATION_GAP_NOTE.to_string(),
        })
        .collect();
    ImpactClosureFacts {
        affected_not_shown: closure.affected_not_shown.clone(),
        coordination_gap,
    }
}

/// Assemble the structured [`ReviewBriefOutput`] for an audit result. Pure: no
/// timestamps, no randomness, so two runs over the same tree serialize
/// byte-identically.
#[must_use]
pub fn build_brief_output(result: &AuditResult) -> ReviewBriefOutput {
    let triage = build_triage(result);
    let closure = result
        .check
        .as_ref()
        .and_then(|c| c.impact_closure.as_ref());
    let deltas = result.review_deltas.clone().unwrap_or_default();
    let mut graph_facts = result.check.as_ref().map_or_else(
        || GraphFacts {
            exports_added: 0,
            api_width_delta: 0,
            reachable_from: Vec::new(),
            boundaries_touched: Vec::new(),
        },
        |check| derive_graph_facts(&check.results, closure),
    );
    // E3 fills the previously-stubbed export facts from the exports-aware delta:
    // `exports_added` / `api_width_delta` count the public-API surface the change
    // widened, not raw internal churn.
    let added = deltas.public_api_added.len();
    graph_facts.exports_added = added;
    graph_facts.api_width_delta = i64::try_from(added).unwrap_or(i64::MAX);
    let impact_closure = build_impact_closure_facts(result);
    ReviewBriefOutput {
        schema_version: ReviewBriefSchemaVersion::default(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        command: "audit-brief".to_string(),
        triage,
        graph_facts,
        impact_closure,
        deltas,
        weakening: result.weakening_signals.clone(),
        routing: result.routing.clone().unwrap_or_default(),
    }
}

/// Insert the Stage 0 triage object into the brief JSON map.
fn insert_brief_triage_json(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    brief: &ReviewBriefOutput,
) {
    if let Ok(value) = serde_json::to_value(&brief.triage) {
        obj.insert("triage".into(), value);
    }
}

/// Insert the Stage 1 graph-facts object into the brief JSON map.
fn insert_brief_graph_facts_json(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    brief: &ReviewBriefOutput,
) {
    if let Ok(value) = serde_json::to_value(&brief.graph_facts) {
        obj.insert("graph_facts".into(), value);
    }
}

/// Insert the Stage 3 impact-closure object into the brief JSON map.
fn insert_brief_impact_closure_json(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    brief: &ReviewBriefOutput,
) {
    if let Ok(value) = serde_json::to_value(&brief.impact_closure) {
        obj.insert("impact_closure".into(), value);
    }
}

/// Insert the E3 deltas / weakening / routing sections into the brief JSON map.
fn insert_brief_e3_json(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    brief: &ReviewBriefOutput,
) {
    if let Ok(value) = serde_json::to_value(&brief.deltas) {
        obj.insert("deltas".into(), value);
    }
    if let Ok(value) = serde_json::to_value(&brief.weakening) {
        obj.insert("weakening".into(), value);
    }
    if let Ok(value) = serde_json::to_value(&brief.routing) {
        obj.insert("routing".into(), value);
    }
}

/// Insert the reused "subtract" section (dead-code / duplication / complexity)
/// into the brief JSON map, mirroring `fallow audit --format json`. Returns the
/// failing exit code if any sub-payload fails to serialize; the caller maps that
/// to a force-success on the brief path but surfaces the serialization error.
fn insert_brief_subtract_json(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    result: &AuditResult,
) -> Result<(), ExitCode> {
    if let Some(ref check) = result.check {
        crate::audit::insert_audit_dead_code_json(obj, result, check)?;
    }
    if let Some(ref dupes) = result.dupes {
        crate::audit::insert_audit_duplication_json(obj, result, dupes)?;
    }
    if let Some(ref health) = result.health {
        crate::audit::insert_audit_health_json(obj, result, health)?;
    }
    Ok(())
}

/// Build the complete brief JSON value: the versioned brief header, the
/// informational audit verdict header, the triage + graph-facts stages, and the
/// reused subtract section.
fn build_brief_json(result: &AuditResult) -> Result<serde_json::Value, ExitCode> {
    let brief = build_brief_output(result);
    let mut obj = serde_json::Map::new();

    obj.insert(
        "schema_version".into(),
        serde_json::to_value(brief.schema_version).unwrap_or(serde_json::Value::Null),
    );
    obj.insert(
        "version".into(),
        serde_json::Value::String(brief.version.clone()),
    );
    obj.insert(
        "command".into(),
        serde_json::Value::String(brief.command.clone()),
    );

    // The audit verdict and scope are carried informationally so the brief is a
    // superset of the audit header; the verdict never drives the brief exit.
    crate::audit::insert_audit_json_header(&mut obj, result);
    // Re-stamp the brief command over the audit header's `"audit"` / its
    // `SCHEMA_VERSION`, so the document advertises the brief contract.
    obj.insert(
        "schema_version".into(),
        serde_json::to_value(brief.schema_version).unwrap_or(serde_json::Value::Null),
    );
    obj.insert(
        "command".into(),
        serde_json::Value::String(brief.command.clone()),
    );

    insert_brief_triage_json(&mut obj, &brief);
    insert_brief_graph_facts_json(&mut obj, &brief);
    insert_brief_impact_closure_json(&mut obj, &brief);
    insert_brief_e3_json(&mut obj, &brief);
    insert_brief_subtract_json(&mut obj, result)?;

    Ok(serde_json::Value::Object(obj))
}

/// Render the brief as JSON. Always returns `SUCCESS`; a serialization failure
/// surfaces the error but the brief contract still exits 0.
fn print_brief_json(result: &AuditResult) -> ExitCode {
    match build_brief_json(result) {
        Ok(mut output) => {
            crate::output_envelope::apply_root_kind(&mut output, "audit-brief");
            crate::output_envelope::attach_telemetry_meta(&mut output);
            let _ = crate::report::emit_json(&output, "audit-brief");
            ExitCode::SUCCESS
        }
        Err(_) => ExitCode::SUCCESS,
    }
}

/// Render the brief in human / compact / markdown form: a short orientation
/// header (scope, risk, effort, boundaries) followed by the same findings
/// sections `fallow audit` prints.
fn print_brief_human(result: &AuditResult, quiet: bool, explain: bool) {
    let brief = build_brief_output(result);

    if !quiet {
        eprintln!();
        eprintln!(
            "Review brief: {} changed file{} vs {} \u{00b7} risk {} \u{00b7} effort {}",
            result.changed_files_count,
            crate::report::plural(result.changed_files_count),
            result.base_ref,
            risk_label(brief.triage.risk_class),
            effort_label(brief.triage.review_effort),
        );
        if !brief.graph_facts.boundaries_touched.is_empty() {
            eprintln!(
                "  boundaries touched: {}",
                brief.graph_facts.boundaries_touched.join(", ")
            );
        }
        print_impact_closure_human(&brief.impact_closure);
        print_deltas_human(&brief.deltas);
        print_weakening_human(&brief.weakening);
        print_routing_human(&brief.routing);
    }

    // Always render the findings sections so the brief shows WHERE to look, even
    // when the underlying verdict is a fail. Headers stay off (the brief owns its
    // own header line above).
    crate::audit::print_audit_findings(result, quiet, explain, false);
}

/// Print the Stage 3 impact-closure summary on the human brief: the count of
/// affected-but-not-shown files and each coordination gap (the precise
/// inter-module attention pointer). Caller has already gated on `!quiet`.
fn print_impact_closure_human(closure: &ImpactClosureFacts) {
    if !closure.affected_not_shown.is_empty() {
        eprintln!(
            "  impact closure: {} file{} affected beyond the diff",
            closure.affected_not_shown.len(),
            crate::report::plural(closure.affected_not_shown.len()),
        );
    }
    for gap in &closure.coordination_gap {
        eprintln!(
            "  coordination gap: {} consumes {} from {} (not in this diff)",
            gap.consumer_file,
            gap.consumed_symbols.join(", "),
            gap.changed_file,
        );
    }
}

/// Print the E3 diff-aware deltas (6.A): boundary/cycle introduced and the
/// exports-aware public-API surface delta (batch-consolidated per R1). Caller
/// has already gated on `!quiet`.
fn print_deltas_human(deltas: &ReviewDeltas) {
    for edge in &deltas.boundary_introduced {
        eprintln!("  new boundary edge: {edge} (not present at base)");
    }
    for cycle in &deltas.cycle_introduced {
        eprintln!("  new circular dependency: {cycle} (not present at base)");
    }
    if !deltas.public_api_added.is_empty() {
        eprintln!(
            "  public API surface widened by {} export{} (exports-aware)",
            deltas.public_api_added.len(),
            crate::report::plural(deltas.public_api_added.len()),
        );
    }
}

/// Print the E3 weakening signals (6.F headline). Advisory, reviewer-private.
fn print_weakening_human(signals: &[crate::audit::weakening::WeakeningSignal]) {
    if signals.is_empty() {
        return;
    }
    eprintln!(
        "  weakening signals ({}, reviewer-private, advisory):",
        signals.len()
    );
    for signal in signals {
        eprintln!(
            "    {}: {} in {}",
            weakening_label(signal.kind),
            signal.evidence,
            signal.file,
        );
    }
}

/// Print the E3 ownership routing (6.D): per-unit expert + bus-factor flag.
fn print_routing_human(routing: &crate::audit::routing::RoutingFacts) {
    for unit in &routing.units {
        if unit.expert.is_empty() {
            continue;
        }
        let bus = if unit.bus_factor_one {
            " (bus-factor 1)"
        } else {
            ""
        };
        eprintln!(
            "  review {}: ask {}{bus}",
            unit.file,
            unit.expert.join(", "),
        );
    }
}

fn weakening_label(kind: crate::audit::weakening::WeakeningKind) -> &'static str {
    use crate::audit::weakening::WeakeningKind;
    match kind {
        WeakeningKind::TestWeakened => "test weakened",
        WeakeningKind::ThresholdLowered => "threshold lowered",
        WeakeningKind::SuppressionAdded => "suppression added",
        WeakeningKind::SecurityCheckRemoved => "security check removed",
    }
}

fn risk_label(risk: RiskClass) -> &'static str {
    match risk {
        RiskClass::Low => "low",
        RiskClass::Medium => "medium",
        RiskClass::High => "high",
    }
}

fn effort_label(effort: ReviewEffort) -> &'static str {
    match effort {
        ReviewEffort::Glance => "glance",
        ReviewEffort::Review => "review",
        ReviewEffort::DeepDive => "deep-dive",
    }
}

/// Print the brief and return an exit code that is ALWAYS `SUCCESS`.
///
/// This is the exit-0 seam: `fallow review` (and `fallow audit --brief`) never
/// gate on the audit verdict. The verdict is still carried in the JSON output
/// informationally. Format dispatch mirrors `print_audit_result`, but every arm
/// forces success: JSON renders the brief envelope; human / compact / markdown
/// render the brief orientation header plus findings; any other format
/// (SARIF, CodeClimate, PR/review envelopes, badge) is rendered through the
/// standard audit path and then forced to success so the format stays usable
/// without re-implementing it for the brief.
#[must_use]
pub fn print_brief_result(result: &AuditResult, quiet: bool, explain: bool) -> ExitCode {
    use fallow_config::OutputFormat;

    match result.output {
        OutputFormat::Json => print_brief_json(result),
        OutputFormat::Human | OutputFormat::Compact | OutputFormat::Markdown => {
            print_brief_human(result, quiet, explain);
            ExitCode::SUCCESS
        }
        _ => {
            // For machine/CI formats not specific to the brief, delegate to the
            // standard audit renderer for the body, then force success: the
            // brief invariant is exit-0 regardless of verdict.
            let _ = crate::audit::print_audit_result(result, quiet, explain);
            ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use fallow_config::{AuditGate, OutputFormat};

    use crate::audit::{AuditAttribution, AuditResult, AuditSummary, AuditVerdict};

    use super::*;

    fn audit_result(verdict: AuditVerdict, output: OutputFormat) -> AuditResult {
        AuditResult {
            verdict,
            summary: AuditSummary {
                dead_code_issues: 0,
                dead_code_has_errors: false,
                complexity_findings: 0,
                max_cyclomatic: None,
                duplication_clone_groups: 0,
            },
            attribution: AuditAttribution {
                gate: AuditGate::NewOnly,
                ..AuditAttribution::default()
            },
            base_snapshot: None,
            base_snapshot_skipped: false,
            changed_files_count: 0,
            changed_files: Vec::new(),
            base_ref: "origin/main".to_string(),
            base_description: None,
            head_sha: None,
            output,
            performance: false,
            check: None,
            dupes: None,
            health: None,
            elapsed: Duration::ZERO,
            review_deltas: None,
            weakening_signals: Vec::new(),
            routing: None,
        }
    }

    #[test]
    fn brief_mode_always_returns_success_even_when_verdict_is_fail() {
        // Human path.
        let human = audit_result(AuditVerdict::Fail, OutputFormat::Human);
        assert_eq!(print_brief_result(&human, true, false), ExitCode::SUCCESS);

        // JSON path.
        let json = audit_result(AuditVerdict::Fail, OutputFormat::Json);
        assert_eq!(print_brief_result(&json, true, false), ExitCode::SUCCESS);
    }

    #[test]
    fn brief_json_validates_against_audit_brief_schema_variant() {
        let result = audit_result(AuditVerdict::Fail, OutputFormat::Json);
        let mut value = build_brief_json(&result).expect("brief json must build");
        crate::output_envelope::apply_root_kind(&mut value, "audit-brief");

        assert_eq!(value["kind"], "audit-brief");
        assert_eq!(value["command"], "audit-brief");
        assert_eq!(value["schema_version"], REVIEW_BRIEF_SCHEMA_VERSION);
    }

    #[test]
    fn brief_json_is_byte_identical_on_repeated_serialization() {
        // `elapsed: Duration::ZERO` and no telemetry: the brief JSON carries no
        // timestamps or randomness, so two builds serialize byte-identically.
        let result = audit_result(AuditVerdict::Warn, OutputFormat::Json);
        let first = build_brief_json(&result).expect("first build");
        let second = build_brief_json(&result).expect("second build");
        let first_str = serde_json::to_string_pretty(&first).expect("serialize first");
        let second_str = serde_json::to_string_pretty(&second).expect("serialize second");
        assert_eq!(first_str, second_str);
    }

    #[test]
    fn risk_class_thresholds_are_pure_functions_of_size() {
        assert_eq!(classify_risk(0, None), RiskClass::Low);
        assert_eq!(classify_risk(RISK_MEDIUM_FILES, None), RiskClass::Medium);
        assert_eq!(classify_risk(RISK_HIGH_FILES, None), RiskClass::High);
        assert_eq!(classify_risk(1, Some(RISK_HIGH_LINES)), RiskClass::High);
        assert_eq!(review_effort_for(RiskClass::High), ReviewEffort::DeepDive);
    }

    #[test]
    fn brief_json_includes_empty_impact_closure_when_no_graph_retained() {
        // check: None -> no closure; the impact_closure object must still be
        // present and empty so consumers can rely on its presence.
        let result = audit_result(AuditVerdict::Warn, OutputFormat::Json);
        let value = build_brief_json(&result).expect("brief json must build");
        assert!(value.get("impact_closure").is_some(), "{value}");
        assert_eq!(
            value["impact_closure"]["affected_not_shown"],
            serde_json::json!([])
        );
        assert_eq!(
            value["impact_closure"]["coordination_gap"],
            serde_json::json!([])
        );
    }

    #[test]
    fn derive_graph_facts_populates_reachable_from_from_closure() {
        use fallow_core::graph::{CoordinationGapPaths, ImpactClosurePaths};
        let results = AnalysisResults::default();
        let closure = ImpactClosurePaths {
            in_diff: vec!["src/core.ts".to_string()],
            affected_not_shown: vec!["src/app.ts".to_string(), "src/mid.ts".to_string()],
            coordination_gap: vec![CoordinationGapPaths {
                changed_file: "src/core.ts".to_string(),
                consumer_file: "src/mid.ts".to_string(),
                consumed_symbols: vec!["compute".to_string()],
            }],
        };
        let facts = derive_graph_facts(&results, Some(&closure));
        assert_eq!(
            facts.reachable_from,
            vec!["src/app.ts".to_string(), "src/mid.ts".to_string()]
        );
    }

    #[test]
    fn coordination_gap_fact_carries_honest_scope_note() {
        let gap = CoordinationGapFact {
            changed_file: "src/core.ts".to_string(),
            consumer_file: "src/mid.ts".to_string(),
            consumed_symbols: vec!["compute".to_string()],
            note: COORDINATION_GAP_NOTE.to_string(),
        };
        assert!(gap.note.contains("attention pointer"));
        assert!(gap.note.contains("not a correctness proof"));
    }
}
