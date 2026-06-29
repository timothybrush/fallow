use super::*;
use fallow_output::{CssAnalyticsReport, CssAnalyticsSummary};

/// Assert two scores / penalties match within rounding tolerance. The scoring
/// pipeline rounds to one decimal, so an exact float compare is both lint-noisy
/// (`clippy::float_cmp`) and brittle; a 0.05 window is well under one rounding
/// step.
fn approx(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 0.05,
        "expected ~{expected}, got {actual}"
    );
}

/// A CSS report whose summary is all-zero (every signal clean) but which is a
/// real, non-empty report (one analyzed stylesheet).
fn clean_report() -> CssAnalyticsReport {
    CssAnalyticsReport {
        files: Vec::new(),
        summary: CssAnalyticsSummary {
            files_analyzed: 1,
            total_declarations: 50,
            ..CssAnalyticsSummary::default()
        },
        scoped_unused: Vec::new(),
        unreferenced_keyframes: Vec::new(),
        undefined_keyframes: Vec::new(),
        duplicate_declaration_blocks: Vec::new(),
        tailwind_arbitrary_values: Vec::new(),
        unused_at_rules: Vec::new(),
        unresolved_class_references: Vec::new(),
        unreferenced_css_classes: Vec::new(),
        unused_font_faces: Vec::new(),
        unused_theme_tokens: Vec::new(),
        font_size_unit_mix: None,
    }
}

#[test]
fn clean_report_scores_100_grade_a() {
    let styling = compute_styling_health(&clean_report());
    assert_eq!(styling.formula_version, STYLING_HEALTH_FORMULA_VERSION);
    approx(styling.score, 100.0);
    assert_eq!(styling.grade, "A");
    let p = &styling.penalties;
    approx(p.duplication, 0.0);
    approx(p.dead_surface, 0.0);
    approx(p.broken_references, 0.0);
    approx(p.token_erosion, 0.0);
    approx(p.structural, 0.0);
}

#[test]
fn duplication_penalty_scales_and_caps() {
    // 5 removable declarations out of 100 = 5% -> 5/100 * 200 = 10pt.
    let mut report = clean_report();
    report.summary.total_declarations = 100;
    report.summary.duplicate_declarations_total = 5;
    let styling = compute_styling_health(&report);
    approx(styling.penalties.duplication, 10.0);

    // 20 removable out of 100 = 20% -> 40pt uncapped, capped at 20pt.
    report.summary.duplicate_declarations_total = 20;
    let styling = compute_styling_health(&report);
    approx(styling.penalties.duplication, DUPLICATION_CAP);
}

#[test]
fn dead_surface_penalty_normalizes_per_stylesheet_and_caps() {
    // 2 dead entities over 2 stylesheets = 1 per sheet -> 10pt.
    let mut report = clean_report();
    report.summary.files_analyzed = 2;
    report.summary.unreferenced_css_classes = 1;
    report.summary.unused_theme_tokens = 1;
    let styling = compute_styling_health(&report);
    approx(styling.penalties.dead_surface, 10.0);

    // 10 dead entities over 1 stylesheet -> 100pt uncapped, capped at 20pt.
    report.summary.files_analyzed = 1;
    report.summary.unreferenced_css_classes = 4;
    report.summary.unused_theme_tokens = 3;
    report.summary.unused_property_registrations = 1;
    report.summary.unused_layers = 1;
    report.summary.unused_font_faces = 1;
    let styling = compute_styling_health(&report);
    approx(styling.penalties.dead_surface, DEAD_SURFACE_CAP);
}

#[test]
fn broken_references_penalty_scales_and_caps() {
    // 2 broken refs (1 class + 1 undefined keyframe) -> 6pt.
    let mut report = clean_report();
    report.summary.unresolved_class_references = 1;
    report.summary.keyframes_undefined = 1;
    let styling = compute_styling_health(&report);
    approx(styling.penalties.broken_references, 6.0);

    // 10 broken refs -> 30pt uncapped, capped at 15pt.
    report.summary.unresolved_class_references = 10;
    report.summary.keyframes_undefined = 0;
    let styling = compute_styling_health(&report);
    approx(styling.penalties.broken_references, BROKEN_REFERENCES_CAP);
}

#[test]
fn token_erosion_penalty_scales_and_caps() {
    // 3 font-size units (1 over the baseline of 2) -> 3pt; +1 arbitrary token = 4pt.
    let mut report = clean_report();
    report.summary.font_size_units_used = 3;
    report.summary.tailwind_arbitrary_values = 1;
    let styling = compute_styling_health(&report);
    approx(styling.penalties.token_erosion, 4.0);

    // Below/at baseline units alone contribute nothing.
    report.summary.font_size_units_used = 2;
    report.summary.tailwind_arbitrary_values = 0;
    let styling = compute_styling_health(&report);
    approx(styling.penalties.token_erosion, 0.0);

    // Many arbitrary tokens cap the category at 10pt.
    report.summary.tailwind_arbitrary_values = 50;
    let styling = compute_styling_health(&report);
    approx(styling.penalties.token_erosion, TOKEN_EROSION_CAP);
}

#[test]
fn structural_penalty_uses_important_density_and_nesting() {
    // 15% !important (10pt over the 5% floor) but nesting at the floor.
    let mut report = clean_report();
    report.summary.total_declarations = 100;
    report.summary.important_declarations = 15;
    report.summary.max_nesting_depth = 4;
    let styling = compute_styling_health(&report);
    approx(styling.penalties.structural, 10.0);

    // Deep nesting alone: depth 7 is 3 over the floor of 4 -> 3pt.
    report.summary.important_declarations = 5; // exactly at the floor -> 0
    report.summary.max_nesting_depth = 7;
    let styling = compute_styling_health(&report);
    approx(styling.penalties.structural, 3.0);
}

#[test]
fn penalties_compound_into_the_score() {
    // A report that trips every category at a known amount; the score is
    // 100 minus the sum of the (capped) per-category penalties.
    let mut report = clean_report();
    report.summary.total_declarations = 100;
    report.summary.duplicate_declarations_total = 5; // 10pt duplication
    report.summary.files_analyzed = 1;
    report.summary.unreferenced_css_classes = 1; // 10pt dead_surface (1/1*10)
    report.summary.unresolved_class_references = 1; // 3pt broken_references
    report.summary.font_size_units_used = 3; // 3pt token_erosion
    report.summary.important_declarations = 15; // 10pt structural
    report.summary.max_nesting_depth = 4;
    let styling = compute_styling_health(&report);
    let p = &styling.penalties;
    approx(p.duplication, 10.0);
    approx(p.dead_surface, 10.0);
    approx(p.broken_references, 3.0);
    approx(p.token_erosion, 3.0);
    approx(p.structural, 10.0);
    // 100 - (10 + 10 + 3 + 3 + 10) = 64.
    approx(styling.score, 64.0);
    assert_eq!(styling.grade, "C");
}

#[test]
fn score_floors_at_rubric_minimum_for_pathological_report() {
    let mut report = clean_report();
    report.summary.total_declarations = 100;
    report.summary.duplicate_declarations_total = 100; // capped 20
    report.summary.files_analyzed = 1;
    report.summary.unreferenced_css_classes = 100; // capped 20
    report.summary.unresolved_class_references = 100; // capped 15
    report.summary.tailwind_arbitrary_values = 100; // capped 10
    report.summary.important_declarations = 100; // capped 10
    report.summary.max_nesting_depth = 20;
    let styling = compute_styling_health(&report);
    // Rubric floor is 25.0 (100 minus the sum of all per-category caps); the
    // `.clamp(0.0, 100.0)` is a defensive guard for any future uncapped category
    // and cannot fire under the current rubric.
    approx(styling.score, 25.0);
    assert!(styling.score >= 0.0);
    assert_eq!(styling.grade, "F");
}

#[test]
fn apply_styling_penalties_clamps_below_zero() {
    // Penalties whose sum exceeds 100 can only arise from an uncapped category;
    // construct that case directly to exercise the clamp branch in
    // `apply_styling_penalties`, which the capped rubric can never reach.
    let penalties = StylingHealthPenalties {
        duplication: 40.0,
        dead_surface: 40.0,
        broken_references: 30.0,
        token_erosion: 20.0,
        structural: 20.0,
    };
    // 100 - 150 = -50, clamped to 0.0.
    let score = apply_styling_penalties(&penalties);
    #[expect(
        clippy::float_cmp,
        reason = "the clamp lower bound is an exact 0.0 literal, so an exact compare is correct here"
    )]
    {
        assert_eq!(score, 0.0);
    }
}
