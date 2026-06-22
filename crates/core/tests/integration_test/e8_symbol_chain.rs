//! E8 done-condition tests for `fallow trace` symbol-level call chains.
//!
//! 1. The caller set matches the hand-computed import-symbol caller set.
//! 2. An unresolved callee is REPORTED, not silently dropped.
//!
//! (The third done-condition, "absent from focus-map ranking inputs", is a
//! structural assertion on the E7 `FocusScore` and lives in the CLI crate
//! where `audit_focus` is defined.)

use super::common::{create_config, fixture_path};
use fallow_core::trace_chain::{
    SymbolChainQuery, TraceDirections, UnresolvedReason, trace_symbol_chain,
};

/// Run the pipeline retaining the graph + modules so the symbol-chain walk has
/// both the import-symbol edges and the parsed module info it needs.
#[expect(
    deprecated,
    reason = "the trace walk needs a retained graph + modules; analyze_retaining_modules is the workspace path-dependency API"
)]
fn trace_fixture(
    fixture: &str,
    file: &str,
    symbol: &str,
    directions: TraceDirections,
    depth: u32,
) -> fallow_core::trace_chain::SymbolChainTrace {
    let config = create_config(fixture_path(fixture));
    let output = fallow_core::analyze_retaining_modules(&config, true, true)
        .expect("analysis should succeed");
    let graph = output.graph.as_ref().expect("graph should be retained");
    let modules = output
        .modules
        .as_deref()
        .expect("modules should be retained");
    trace_symbol_chain(
        graph,
        modules,
        &config.root,
        SymbolChainQuery {
            file,
            symbol,
            depth,
            directions,
        },
    )
    .expect("symbol should be found in the graph")
}

/// Normalize a hop's file path to a forward-slash string for cross-platform
/// comparison.
fn hop_files(hops: &[fallow_core::trace_chain::ChainHop]) -> std::collections::BTreeSet<String> {
    hops.iter()
        .map(|h| h.file.to_string_lossy().replace('\\', "/"))
        .collect()
}

#[test]
fn caller_set_matches_hand_computed_import_symbol_callers() {
    let trace = trace_fixture(
        "e8-symbol-chain",
        "src/format.ts",
        "formatDate",
        TraceDirections {
            callers: true,
            callees: false,
        },
        1,
    );

    assert!(trace.symbol_found, "formatDate is an export of format.ts");
    assert!(
        trace.best_effort,
        "symbol-level chains are labeled best-effort"
    );

    let callers = trace.callers.expect("callers were requested");
    let actual = hop_files(&callers);

    // Hand-computed: exactly the two files that `import { formatDate } from
    // "./format"` -> report.ts and middle.ts. index.ts does NOT import
    // formatDate directly, so it is NOT in the depth-1 caller set.
    let expected: std::collections::BTreeSet<String> =
        ["src/report.ts".to_string(), "src/middle.ts".to_string()]
            .into_iter()
            .collect();
    assert_eq!(
        actual, expected,
        "caller set must equal the hand-computed set"
    );

    // Each direct caller hop imports it under the name `formatDate` at depth 1.
    for hop in &callers {
        assert_eq!(hop.imported_as, "formatDate");
        assert_eq!(hop.local_name, "formatDate");
        assert_eq!(hop.depth, 1);
        assert!(!hop.type_only);
    }
}

#[test]
fn unresolved_callees_are_reported_not_dropped() {
    let trace = trace_fixture(
        "e8-symbol-chain",
        "src/report.ts",
        "buildReport",
        TraceDirections {
            callers: false,
            callees: true,
        },
        1,
    );

    assert!(trace.symbol_found, "buildReport is an export of report.ts");

    let unresolved = trace
        .unresolved_callees
        .expect("callees were requested, so unresolved_callees is present");

    let callees: Vec<&str> = unresolved.iter().map(|u| u.callee.as_str()).collect();

    // `localHelper` (same-module local) and `parseInt` (global) are callees that
    // do NOT resolve to an import-symbol edge. They MUST be reported, not
    // silently dropped.
    assert!(
        callees.contains(&"localHelper"),
        "the local helper callee must be reported as unresolved, got {callees:?}"
    );
    assert!(
        callees.contains(&"parseInt"),
        "the global callee must be reported as unresolved, got {callees:?}"
    );

    // `formatDate` resolves to an import-symbol edge, so it is a RESOLVED callee,
    // never in the unresolved list.
    assert!(
        !callees.contains(&"formatDate"),
        "an imported callee resolves to an edge and is not unresolved, got {callees:?}"
    );

    // The bare-identifier callees classify as LocalOrGlobal.
    let local_helper = unresolved
        .iter()
        .find(|u| u.callee == "localHelper")
        .unwrap();
    assert_eq!(local_helper.reason, UnresolvedReason::LocalOrGlobal);

    // The resolved-callee hop for formatDate is present in the down-walk.
    let callees_hops = trace.callees.expect("callees were requested");
    let resolved_files = hop_files(&callees_hops);
    assert!(
        resolved_files.contains("src/format.ts"),
        "the resolved import-symbol callee edge to format.ts must be present, got {resolved_files:?}"
    );
}
