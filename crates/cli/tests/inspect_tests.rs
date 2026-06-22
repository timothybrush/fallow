#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "integration tests keep fixture setup concise"
)]

#[path = "common/mod.rs"]
mod common;

use common::{parse_json, run_fallow_in_root};
use tempfile::tempdir;

fn write_project(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{"name":"inspect-fixture","type":"module"}"#,
    )
    .unwrap();
    std::fs::write(root.join("tsconfig.json"), r#"{"include":["src"]}"#).unwrap();
    std::fs::write(root.join(".fallowrc.json"), r#"{"entry":["src/index.ts"]}"#).unwrap();
    std::fs::write(
        root.join("src/index.ts"),
        "import { fetchUser } from './api';\nexport const boot = () => fetchUser('1');\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/api.ts"),
        "export const fetchUser = (id: string) => ({ id });\n",
    )
    .unwrap();
}

#[test]
fn inspect_file_outputs_typed_evidence_bundle() {
    let dir = tempdir().unwrap();
    write_project(dir.path());

    let output = run_fallow_in_root(
        "inspect",
        dir.path(),
        &["--file", "src/api.ts", "--format", "json", "--quiet"],
    );
    assert_eq!(output.code, 0, "inspect should exit 0: {}", output.stderr);

    let json = parse_json(&output);
    assert_eq!(json["kind"].as_str(), Some("inspect_target"));
    assert_eq!(json["target"]["type"].as_str(), Some("file"));
    assert_eq!(json["target"]["file"].as_str(), Some("src/api.ts"));
    assert_eq!(json["identity"]["file"].as_str(), Some("src/api.ts"));
    assert_eq!(
        json["evidence"]["trace_file"]["status"].as_str(),
        Some("ok")
    );
    assert_eq!(
        json["evidence"]["dead_code"]["scope"].as_str(),
        Some("file")
    );
    assert_eq!(
        json["evidence"]["duplication"]["scope"].as_str(),
        Some("project_filtered_to_file")
    );
}

#[test]
fn inspect_file_accepts_absolute_path_inside_root() {
    let dir = tempdir().unwrap();
    write_project(dir.path());

    let file = dir.path().join("src/api.ts");
    let output = run_fallow_in_root(
        "inspect",
        dir.path(),
        &[
            "--file",
            file.to_str().unwrap(),
            "--format",
            "json",
            "--quiet",
        ],
    );
    assert_eq!(output.code, 0, "inspect should exit 0: {}", output.stderr);

    let json = parse_json(&output);
    assert_eq!(json["target"]["file"].as_str(), Some("src/api.ts"));
    assert_eq!(json["identity"]["file"].as_str(), Some("src/api.ts"));
}

#[test]
fn inspect_symbol_outputs_trace_export_section() {
    let dir = tempdir().unwrap();
    write_project(dir.path());

    let output = run_fallow_in_root(
        "inspect",
        dir.path(),
        &[
            "--symbol",
            "src/api.ts:fetchUser",
            "--format",
            "json",
            "--quiet",
        ],
    );
    assert_eq!(output.code, 0, "inspect should exit 0: {}", output.stderr);

    let json = parse_json(&output);
    assert_eq!(json["kind"].as_str(), Some("inspect_target"));
    assert_eq!(json["target"]["type"].as_str(), Some("symbol"));
    assert_eq!(json["target"]["export_name"].as_str(), Some("fetchUser"));
    assert_eq!(json["identity"]["export_name"].as_str(), Some("fetchUser"));
    assert_eq!(
        json["evidence"]["trace_export"]["status"].as_str(),
        Some("ok")
    );
    assert!(
        json["warnings"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
}

#[test]
fn inspect_human_output_includes_evidence_summary() {
    let dir = tempdir().unwrap();
    write_project(dir.path());

    let output = run_fallow_in_root("inspect", dir.path(), &["--file", "src/api.ts"]);
    assert_eq!(output.code, 0, "inspect should exit 0: {}", output.stderr);

    assert!(output.stdout.contains("Evidence"));
    assert!(output.stdout.contains("trace_file: ok [file]"));
    assert!(
        output
            .stdout
            .contains("duplication: ok [project filtered to file]")
    );
}

#[test]
fn inspect_file_includes_impact_closure_evidence() {
    let dir = tempdir().unwrap();
    write_project(dir.path());

    // index.ts imports fetchUser from api.ts. Inspecting api.ts as the seed yields
    // an impact closure whose affected-not-shown set is {src/index.ts} and whose
    // coordination gap fires on index.ts consuming fetchUser (outside the seed).
    let output = run_fallow_in_root(
        "inspect",
        dir.path(),
        &["--file", "src/api.ts", "--format", "json", "--quiet"],
    );
    assert_eq!(output.code, 0, "inspect should exit 0: {}", output.stderr);

    let json = parse_json(&output);
    let closure = &json["evidence"]["impact_closure"];
    assert_eq!(closure["status"].as_str(), Some("ok"), "{json}");
    assert_eq!(closure["scope"].as_str(), Some("project_filtered_to_file"));
    let data = &closure["data"];
    assert_eq!(data["seed"].as_str(), Some("src/api.ts"));
    let affected: Vec<&str> = data["affected_not_shown"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(affected, vec!["src/index.ts"]);
    let gaps = data["coordination_gap"].as_array().unwrap();
    assert_eq!(gaps.len(), 1, "exactly one coordination gap: {data}");
    assert_eq!(gaps[0]["consumer_file"].as_str(), Some("src/index.ts"));
    let symbols: Vec<&str> = gaps[0]["consumed_symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(symbols, vec!["fetchUser"]);
}

#[test]
fn dead_code_impact_closure_flag_emits_closure_json() {
    let dir = tempdir().unwrap();
    write_project(dir.path());

    let output = run_fallow_in_root(
        "dead-code",
        dir.path(),
        &[
            "--impact-closure",
            "src/api.ts",
            "--format",
            "json",
            "--quiet",
        ],
    );
    assert_eq!(
        output.code, 0,
        "impact-closure should exit 0: {}",
        output.stderr
    );

    let json = parse_json(&output);
    assert_eq!(json["seed"].as_str(), Some("src/api.ts"));
    let affected: Vec<&str> = json["affected_not_shown"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(affected, vec!["src/index.ts"]);
    assert_eq!(
        json["coordination_gap"][0]["consumer_file"].as_str(),
        Some("src/index.ts")
    );
    assert!(
        json["coordination_gap"][0]["note"]
            .as_str()
            .is_some_and(|n| n.contains("attention pointer"))
    );
}
