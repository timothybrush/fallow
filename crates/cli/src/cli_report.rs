//! `fallow report --from <results.json>`: render an EXISTING fallow JSON
//! envelope in another format without re-running analysis (the analyze-once
//! flow: `fallow --format json -o results.json`, then one `report` call per
//! rendered surface).
//!
//! v1 supports only the GitHub-native text formats; SARIF and markdown
//! re-rendering from a saved envelope is a recorded follow-up. Dispatch is on
//! the envelope's `kind` field, so any envelope produced by `--format json`
//! (dead-code, dupes, health, audit, security, or the bare combined run)
//! renders byte-identically to the direct `--format` run.

use std::path::Path;
use std::process::ExitCode;

use fallow_config::OutputFormat;

use crate::report::github_annotations::{self, EnvelopeKind};
use crate::report::github_summary;
use crate::telemetry;

/// Run `fallow report --from <file>` with the global `--format` and `--root`.
pub fn run_report(from: &Path, output: OutputFormat, root: &Path) -> ExitCode {
    let summary = match output {
        OutputFormat::GithubAnnotations => false,
        OutputFormat::GithubSummary => true,
        _ => {
            return crate::emit_known_failure(
                "fallow report supports --format github-annotations or github-summary only \
                 (re-rendering saved envelopes as sarif or markdown is a recorded follow-up)",
                2,
                output,
                telemetry::FailureReason::UnsupportedFormat,
            );
        }
    };
    let envelope = match load_envelope(from, output) {
        Ok(envelope) => envelope,
        Err(code) => return code,
    };
    let kind = match envelope_kind(&envelope, from, output) {
        Ok(kind) => kind,
        Err(code) => return code,
    };
    if summary {
        github_summary::print_summary(kind, &envelope, root)
    } else {
        github_annotations::print_annotations(kind, &envelope, root)
    }
}

fn load_envelope(from: &Path, output: OutputFormat) -> Result<serde_json::Value, ExitCode> {
    let source = std::fs::read_to_string(from).map_err(|err| {
        crate::emit_known_failure(
            &format!("failed to read {}: {err}", from.display()),
            2,
            output,
            telemetry::FailureReason::Validation,
        )
    })?;
    serde_json::from_str(&source).map_err(|err| {
        crate::emit_known_failure(
            &format!(
                "{} is not valid JSON ({err}); generate it with `fallow ... --format json`",
                from.display()
            ),
            2,
            output,
            telemetry::FailureReason::Validation,
        )
    })
}

fn envelope_kind(
    envelope: &serde_json::Value,
    from: &Path,
    output: OutputFormat,
) -> Result<EnvelopeKind, ExitCode> {
    let Some(kind) = envelope.get("kind").and_then(serde_json::Value::as_str) else {
        return Err(crate::emit_known_failure(
            &format!(
                "{} is not a fallow results envelope (missing top-level `kind`); \
                 generate it with `fallow ... --format json`",
                from.display()
            ),
            2,
            output,
            telemetry::FailureReason::Validation,
        ));
    };
    parse_envelope_kind(kind).ok_or_else(|| {
        crate::emit_known_failure(
            &format!(
                "unsupported envelope kind `{kind}` in {}; fallow report renders dead-code, \
                 dupes, health, audit, security, and combined envelopes",
                from.display()
            ),
            2,
            output,
            telemetry::FailureReason::Validation,
        )
    })
}

/// Map the `--format json` root `kind` onto the renderer dispatch. The fix
/// envelope has no `kind` field, so fix output is not re-renderable here (use
/// `fallow fix --format github-summary` directly).
fn parse_envelope_kind(kind: &str) -> Option<EnvelopeKind> {
    match kind {
        "dead-code" => Some(EnvelopeKind::DeadCode),
        "dupes" => Some(EnvelopeKind::Dupes),
        "health" => Some(EnvelopeKind::Health),
        "audit" => Some(EnvelopeKind::Audit),
        "security" => Some(EnvelopeKind::Security),
        "combined" => Some(EnvelopeKind::Combined),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_envelope_kind_covers_supported_kinds() {
        assert_eq!(
            parse_envelope_kind("dead-code"),
            Some(EnvelopeKind::DeadCode)
        );
        assert_eq!(parse_envelope_kind("dupes"), Some(EnvelopeKind::Dupes));
        assert_eq!(parse_envelope_kind("health"), Some(EnvelopeKind::Health));
        assert_eq!(parse_envelope_kind("audit"), Some(EnvelopeKind::Audit));
        assert_eq!(
            parse_envelope_kind("security"),
            Some(EnvelopeKind::Security)
        );
        assert_eq!(
            parse_envelope_kind("combined"),
            Some(EnvelopeKind::Combined)
        );
    }

    #[test]
    fn parse_envelope_kind_rejects_unknown_and_grouped_kinds() {
        assert_eq!(parse_envelope_kind("dead-code-grouped"), None);
        assert_eq!(parse_envelope_kind("feature-flags"), None);
        assert_eq!(parse_envelope_kind(""), None);
    }
}
