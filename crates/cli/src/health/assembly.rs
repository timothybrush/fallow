use super::{HealthOptions, HealthReportAssembly, coverage_intelligence};
use crate::health_types::{HealthReport, HealthSummary};

/// Assemble the final `HealthReport` from all computed data.
#[expect(
    clippy::too_many_lines,
    reason = "report assembly threads every optional health feature into the final envelope; splitting fragments the read-path"
)]
pub(super) fn assemble_health_report(
    opts: &HealthOptions<'_>,
    action_ctx: &crate::health_types::HealthActionContext,
    assembly: HealthReportAssembly,
) -> HealthReport {
    let HealthReportAssembly {
        report_coverage_gaps,
        findings,
        files_analyzed,
        total_functions,
        total_above_threshold,
        max_cyclomatic,
        max_cognitive,
        max_crap,
        files_scored,
        average_maintainability,
        vital_signs,
        health_score,
        score_output,
        hotspots,
        hotspot_summary,
        targets,
        target_thresholds,
        health_trend,
        has_istanbul_coverage,
        runtime_coverage,
        large_functions,
        sev_critical,
        sev_high,
        sev_moderate,
    } = assembly;
    let coverage_gaps = if report_coverage_gaps {
        score_output.as_ref().map(|o| o.coverage.report.clone())
    } else {
        None
    };

    let (ist_matched, ist_total) = score_output
        .as_ref()
        .map_or((0, 0), |o| (o.istanbul_matched, o.istanbul_total));

    let file_scores = if opts.score_only_output {
        Vec::new()
    } else if opts.file_scores {
        let mut scores = score_output.map(|o| o.scores).unwrap_or_default();
        if let Some(top) = opts.top {
            scores.truncate(top);
        }
        scores
    } else {
        Vec::new()
    };

    let (report_hotspots, report_hotspot_summary) = if opts.hotspots {
        (hotspots, hotspot_summary)
    } else {
        (Vec::new(), None)
    };

    let summary_files_scored = if opts.score_only_output || !opts.file_scores {
        None
    } else {
        files_scored
    };
    let summary_average_maintainability = if opts.score_only_output || !opts.file_scores {
        None
    } else {
        average_maintainability
    };
    let summary_coverage_model = if opts.score_only_output {
        None
    } else if opts.file_scores || report_coverage_gaps || opts.hotspots || opts.targets {
        Some(if has_istanbul_coverage {
            crate::health_types::CoverageModel::Istanbul
        } else {
            crate::health_types::CoverageModel::StaticEstimated
        })
    } else {
        None
    };
    let summary_coverage_source_consistency = if opts.score_only_output || !opts.complexity {
        None
    } else {
        crate::health_types::summarize_coverage_source_consistency(
            findings
                .iter()
                .filter_map(|finding| finding.coverage_source),
        )
    };
    let summary_istanbul_matched = if opts.score_only_output || !has_istanbul_coverage {
        None
    } else {
        Some(ist_matched)
    };
    let summary_istanbul_total = if opts.score_only_output || !has_istanbul_coverage {
        None
    } else {
        Some(ist_total)
    };

    let mut report = HealthReport {
        summary: HealthSummary {
            files_analyzed,
            functions_analyzed: total_functions,
            functions_above_threshold: total_above_threshold,
            max_cyclomatic_threshold: max_cyclomatic,
            max_cognitive_threshold: max_cognitive,
            max_crap_threshold: max_crap,
            files_scored: summary_files_scored,
            average_maintainability: summary_average_maintainability,
            coverage_model: summary_coverage_model,
            coverage_source_consistency: summary_coverage_source_consistency,
            istanbul_matched: summary_istanbul_matched,
            istanbul_total: summary_istanbul_total,
            severity_critical_count: sev_critical,
            severity_high_count: sev_high,
            severity_moderate_count: sev_moderate,
        },
        vital_signs: if opts.score_only_output {
            None
        } else {
            Some(vital_signs)
        },
        health_score,
        findings: if opts.complexity {
            findings
                .into_iter()
                .map(|v| crate::health_types::HealthFinding::with_actions(v, action_ctx))
                .collect()
        } else {
            Vec::new()
        },
        file_scores,
        coverage_gaps: if opts.score_only_output {
            None
        } else {
            coverage_gaps
        },
        hotspots: report_hotspots
            .into_iter()
            .map(|h| crate::health_types::HotspotFinding::with_actions(h, opts.root))
            .collect(),
        hotspot_summary: if opts.score_only_output {
            None
        } else {
            report_hotspot_summary
        },
        runtime_coverage,
        coverage_intelligence: None,
        large_functions: if opts.score_only_output {
            Vec::new()
        } else {
            large_functions
        },
        targets: if opts.score_only_output {
            Vec::new()
        } else {
            targets
                .into_iter()
                .map(crate::health_types::RefactoringTargetFinding::with_actions)
                .collect()
        },
        target_thresholds: if opts.score_only_output {
            None
        } else {
            target_thresholds
        },
        health_trend,
        actions_meta: if action_ctx.opts.omit_suppress_line {
            Some(crate::health_types::HealthActionsMeta {
                suppression_hints_omitted: true,
                reason: action_ctx
                    .opts
                    .omit_reason
                    .unwrap_or("unspecified")
                    .to_string(),
                scope: "health-findings".to_string(),
            })
        } else {
            None
        },
    };
    if !opts.score_only_output {
        report.coverage_intelligence = coverage_intelligence::build_coverage_intelligence(
            &report,
            opts.root,
            coverage_intelligence::CoverageIntelligenceContext {
                has_change_scope: opts.changed_since.is_some()
                    || opts.diff_index.is_some()
                    || opts.use_shared_diff_index,
            },
        );
    }
    report
}
