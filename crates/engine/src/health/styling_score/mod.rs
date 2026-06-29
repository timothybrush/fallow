//! Styling-health score: a SECOND health axis derived purely from the structural
//! CSS analytics ([`CssAnalyticsReport`]), orthogonal to the JS/TS code-health
//! score. Mirrors the code-score shape (`vital_signs::compute_health_score`):
//! start at 100, subtract capped per-category penalties, map the result to a
//! letter grade via the shared [`letter_grade`]. The code score is never touched.
//!
//! # Penalty rubric (v1, [`STYLING_HEALTH_FORMULA_VERSION`])
//!
//! The weights below are the provisional first-slice rubric, pending corpus
//! calibration; any recalibration bumps [`STYLING_HEALTH_FORMULA_VERSION`] (the
//! same versioning discipline the code score uses). Every category is capped and
//! evaluated against the analytics that are always present once `--css` ran, so
//! there is no "missing pipeline" partial-credit case to model.
//!
//! | Category | Cap | Signal | Scaling |
//! |---|---|---|---|
//! | `duplication` | 20pt | `summary.duplicate_declarations_total` (removable declarations from copy-paste blocks) | `total / max(total_declarations, 1) * 200`, capped (so ~10% of declarations removable = full 20pt) |
//! | `dead_surface` | 20pt | unreferenced classes + unused `@theme` tokens + unused at-rules + dead `@font-face` | `count_per_stylesheet * 10`, capped (so ~2 dead entities per analyzed stylesheet = full 20pt) |
//! | `broken_references` | 15pt | `unresolved_class_references` + `keyframes_undefined` | `count * 3`, capped (so 5 broken refs = full 15pt) |
//! | `token_erosion` | 10pt | mixed `font-size` units (above 2) + distinct Tailwind arbitrary-value tokens | `(extra_units * 3) + min(arbitrary_tokens, ...)`, capped |
//! | `structural` | 10pt | `!important` density + deep nesting | `(important_pct - 5).clamp * 1` + `(max_nesting - 4).clamp * 1`, capped |
//!
//! All point values are rounded to one decimal, the final score is clamped to
//! `[0, 100]`, matching the code score's `round1` / clamp behaviour.

use fallow_output::{
    CssAnalyticsReport, STYLING_HEALTH_FORMULA_VERSION, StylingHealth, StylingHealthPenalties,
    letter_grade,
};

const DUPLICATION_CAP: f64 = 20.0;
const DEAD_SURFACE_CAP: f64 = 20.0;
const BROKEN_REFERENCES_CAP: f64 = 15.0;
const TOKEN_EROSION_CAP: f64 = 10.0;
const STRUCTURAL_CAP: f64 = 10.0;

/// `!important` density (as a percentage of declarations) below which no
/// structural penalty accrues. A small amount of `!important` is normal.
const IMPORTANT_DENSITY_FLOOR: f64 = 5.0;

/// Style-rule nesting depth at or below which no structural penalty accrues.
/// Shallow nesting is healthy; the penalty grows past this floor.
const NESTING_DEPTH_FLOOR: f64 = 4.0;

/// The number of distinct `font-size` units considered a healthy baseline (e.g.
/// `rem` for type plus `px` for fixed chrome). Each unit beyond this erodes the
/// token system.
const FONT_SIZE_UNIT_BASELINE: u32 = 2;

/// Compute the styling-health score from the structural CSS analytics.
///
/// Mirrors the code score: penalties subtract from a starting 100, the result is
/// rounded and clamped, and the grade reuses the shared [`letter_grade`]
/// thresholds. See the module docs for the per-category rubric (v1).
#[must_use]
pub fn compute_styling_health(report: &CssAnalyticsReport) -> StylingHealth {
    let penalties = compute_styling_penalties(report);
    let score = apply_styling_penalties(&penalties);
    let grade = letter_grade(score);

    StylingHealth {
        formula_version: STYLING_HEALTH_FORMULA_VERSION,
        score,
        grade,
        penalties,
    }
}

fn compute_styling_penalties(report: &CssAnalyticsReport) -> StylingHealthPenalties {
    StylingHealthPenalties {
        duplication: duplication_penalty(report),
        dead_surface: dead_surface_penalty(report),
        broken_references: broken_references_penalty(report),
        token_erosion: token_erosion_penalty(report),
        structural: structural_penalty(report),
    }
}

fn apply_styling_penalties(penalties: &StylingHealthPenalties) -> f64 {
    let mut score = 100.0_f64;
    score -= penalties.duplication;
    score -= penalties.dead_surface;
    score -= penalties.broken_references;
    score -= penalties.token_erosion;
    score -= penalties.structural;
    round1(score).clamp(0.0, 100.0)
}

/// Copy-paste declaration blocks: penalize by the share of all declarations that
/// could be removed by consolidating duplicate blocks. ~10% removable -> full cap.
fn duplication_penalty(report: &CssAnalyticsReport) -> f64 {
    let removable = f64::from(report.summary.duplicate_declarations_total);
    let total = f64::from(report.summary.total_declarations).max(1.0);
    round1((removable / total * 200.0).min(DUPLICATION_CAP))
}

/// Dead styling surface: unreferenced classes, unused `@theme` tokens, unused
/// `@property`/`@layer` at-rules, and dead `@font-face` families, normalized per
/// analyzed stylesheet so a large project is not penalized for absolute counts.
fn dead_surface_penalty(report: &CssAnalyticsReport) -> f64 {
    let s = &report.summary;
    let dead = f64::from(
        s.unreferenced_css_classes
            .saturating_add(s.unused_theme_tokens)
            .saturating_add(s.unused_property_registrations)
            .saturating_add(s.unused_layers)
            .saturating_add(s.unused_font_faces),
    );
    let stylesheets = f64::from(s.files_analyzed).max(1.0);
    round1((dead / stylesheets * 10.0).min(DEAD_SURFACE_CAP))
}

/// Broken references: markup classes one edit from a defined class, plus
/// animations referencing a `@keyframes` defined nowhere. Each is a likely typo
/// or stale rename; 5 broken refs reach the cap.
fn broken_references_penalty(report: &CssAnalyticsReport) -> f64 {
    let s = &report.summary;
    let broken = f64::from(
        s.unresolved_class_references
            .saturating_add(s.keyframes_undefined),
    );
    round1((broken * 3.0).min(BROKEN_REFERENCES_CAP))
}

/// Design-token erosion: mixing `font-size` units past a healthy baseline and
/// Tailwind arbitrary-value bypasses both work against a single source of truth
/// for the scale.
fn token_erosion_penalty(report: &CssAnalyticsReport) -> f64 {
    let s = &report.summary;
    let extra_units = f64::from(
        s.font_size_units_used
            .saturating_sub(FONT_SIZE_UNIT_BASELINE),
    );
    let arbitrary = f64::from(s.tailwind_arbitrary_values);
    round1(extra_units.mul_add(3.0, arbitrary).min(TOKEN_EROSION_CAP))
}

/// Structural smells: `!important` density above a healthy floor and deep
/// style-rule nesting past a shallow floor.
fn structural_penalty(report: &CssAnalyticsReport) -> f64 {
    let s = &report.summary;
    let important_pct = if s.total_declarations > 0 {
        f64::from(s.important_declarations) / f64::from(s.total_declarations) * 100.0
    } else {
        0.0
    };
    let important = (important_pct - IMPORTANT_DENSITY_FLOOR).max(0.0);
    let nesting = (f64::from(s.max_nesting_depth) - NESTING_DEPTH_FLOOR).max(0.0);
    round1((important + nesting).min(STRUCTURAL_CAP))
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests;
