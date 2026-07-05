//! Engine-owned repository reference probes and temporary repo views.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use fallow_config::WorkspaceInfo;

use crate::{EngineError, EngineResult};

/// Resolved base ref for changed-code audit.
#[derive(Debug, Clone)]
pub struct ResolvedAuditBase {
    /// Git ref or SHA used for comparison.
    pub git_ref: String,
    /// Human-readable source of the resolved ref.
    pub description: Option<String>,
}

/// Temporary detached worktree for comparing audit results against a base ref.
#[derive(Debug)]
pub struct TemporaryBaseWorktree {
    repo_root: PathBuf,
    path: PathBuf,
}

impl TemporaryBaseWorktree {
    /// Create a detached base worktree for `base_ref`.
    ///
    /// # Errors
    ///
    /// Returns an engine error when the temp path cannot be generated, `git`
    /// cannot be started, or the worktree cannot be created.
    pub fn create(repo_root: &Path, base_ref: &str) -> EngineResult<Self> {
        let path = base_worktree_path()?;
        let mut command = git_command(repo_root);
        command
            .arg("worktree")
            .arg("add")
            .arg("--detach")
            .arg("--quiet")
            .arg(&path)
            .arg(base_ref);
        let output = command.output().map_err(|err| {
            EngineError::new(format!(
                "could not create a temporary worktree for base ref `{base_ref}`: {err}"
            ))
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(EngineError::new(format!(
                "could not create a temporary worktree for base ref `{base_ref}`: {stderr}"
            )));
        }
        Ok(Self {
            repo_root: repo_root.to_path_buf(),
            path,
        })
    }

    /// Path to the detached worktree.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryBaseWorktree {
    fn drop(&mut self) {
        let mut command = git_command(&self.repo_root);
        command
            .arg("worktree")
            .arg("remove")
            .arg("--force")
            .arg(&self.path);
        let _ = command.output();
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Resolve the analysis root inside a detached base worktree.
#[must_use]
pub fn base_analysis_root(current_root: &Path, base_worktree_root: &Path) -> PathBuf {
    let Some(git_root) = git_toplevel(current_root) else {
        return base_worktree_root.to_path_buf();
    };
    let current_root =
        dunce::canonicalize(current_root).unwrap_or_else(|_| current_root.to_path_buf());
    match current_root.strip_prefix(&git_root) {
        Ok(relative) => base_worktree_root.join(relative),
        Err(_) => base_worktree_root.to_path_buf(),
    }
}

/// Auto-detect the base ref used by changed-code audit.
#[must_use]
pub fn auto_detect_audit_base_ref(root: &Path) -> Option<ResolvedAuditBase> {
    if let Some(upstream) = git_upstream_ref(root) {
        if let Some(sha) = git_merge_base(root, &upstream, "HEAD") {
            return Some(ResolvedAuditBase {
                git_ref: sha,
                description: Some(format!("merge-base with {upstream}")),
            });
        }
        return Some(ResolvedAuditBase {
            description: Some(format!("{upstream} (tip)")),
            git_ref: upstream,
        });
    }

    if let Some(remote_ref) = detect_remote_default_ref(root) {
        if let Some(sha) = git_merge_base(root, &remote_ref, "HEAD") {
            return Some(ResolvedAuditBase {
                git_ref: sha,
                description: Some(format!("merge-base with {remote_ref}")),
            });
        }
        return Some(ResolvedAuditBase {
            description: Some(format!("{remote_ref} (tip)")),
            git_ref: remote_ref,
        });
    }

    for candidate in ["main", "master"] {
        if git_ref_exists(root, candidate) {
            return Some(ResolvedAuditBase {
                git_ref: candidate.to_string(),
                description: Some(format!("local {candidate}")),
            });
        }
    }

    None
}

/// Short SHA for the current HEAD.
#[must_use]
pub fn short_head_sha(root: &Path) -> Option<String> {
    run_git(root, &["rev-parse", "--short", "HEAD"])
}

/// Resolve a concrete `--changed-workspaces` ref for project-level next steps.
///
/// Returns `None` when the project has no workspaces, is not a git repository,
/// or has no resolvable remote default branch.
#[must_use]
pub fn default_workspace_ref(root: &Path) -> Option<String> {
    let workspaces = crate::discover::discover_workspace_packages(root);
    default_workspace_ref_for_workspaces(root, &workspaces)
}

/// Resolve a concrete `--changed-workspaces` ref using existing workspace data.
#[must_use]
pub fn default_workspace_ref_for_workspaces(
    root: &Path,
    workspaces: &[WorkspaceInfo],
) -> Option<String> {
    if workspaces.is_empty() || !crate::churn::is_git_repo(root) {
        return None;
    }
    if let Some(reference) = run_git(
        root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    ) {
        let reference = reference.trim();
        if !reference.is_empty() {
            return Some(reference.to_owned());
        }
    }
    ["origin/main", "origin/master"]
        .into_iter()
        .find(|candidate| git_ref_exists(root, candidate))
        .map(str::to_owned)
}

/// Git identities for the current user in forms useful for self-routing.
///
/// Includes `user.email`, its local-part handle, a GitHub no-reply unwrapped
/// handle when applicable, and `user.name`. Missing config values are ignored.
#[must_use]
pub fn current_user_identities(root: &Path) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(email) = read_git_config(root, "user.email") {
        if let Some((local, _)) = email.split_once('@') {
            ids.push(local.rsplit('+').next().unwrap_or(local).to_owned());
        }
        ids.push(email);
    }
    if let Some(name) = read_git_config(root, "user.name") {
        ids.push(name);
    }
    ids
}

fn read_git_config(root: &Path, key: &str) -> Option<String> {
    let value = run_git(root, &["config", "--get", key])?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn git_ref_exists(root: &Path, reference: &str) -> bool {
    run_git(root, &["rev-parse", "--verify", "--quiet", reference]).is_some()
}

fn git_toplevel(root: &Path) -> Option<PathBuf> {
    run_git(root, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

fn git_upstream_ref(root: &Path) -> Option<String> {
    run_git(
        root,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
}

fn git_merge_base(root: &Path, a: &str, b: &str) -> Option<String> {
    run_git(root, &["merge-base", a, b])
}

fn detect_remote_default_ref(root: &Path) -> Option<String> {
    if let Some(full_ref) = run_git(root, &["symbolic-ref", "refs/remotes/origin/HEAD"])
        && let Some(branch) = full_ref.strip_prefix("refs/remotes/origin/")
    {
        return Some(format!("origin/{branch}"));
    }
    ["origin/main", "origin/master"]
        .into_iter()
        .find(|candidate| git_ref_exists(root, candidate))
        .map(str::to_string)
}

fn base_worktree_path() -> EngineResult<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|err| EngineError::new(format!("system clock before unix epoch: {err}")))?
        .as_nanos();
    Ok(std::env::temp_dir().join(format!("fallow-audit-base-{}-{nanos}", std::process::id())))
}

#[expect(
    clippy::disallowed_methods,
    reason = "canonical engine-owned git spawn wrapper for repository refs"
)]
fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    crate::changed_files::clear_ambient_git_env(&mut command);
    command.arg("-C").arg(root);
    command
}

fn run_git(root: &Path, args: &[&str]) -> Option<String> {
    let output = git_command(root).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn default_workspace_ref_skips_projects_without_workspaces() {
        assert!(default_workspace_ref_for_workspaces(Path::new("/repo"), &[]).is_none());
    }

    #[test]
    fn default_workspace_ref_skips_non_git_workspace_projects() {
        let workspace = WorkspaceInfo {
            root: PathBuf::from("/repo/packages/app"),
            name: "app".to_owned(),
            is_internal_dependency: false,
        };

        assert!(default_workspace_ref_for_workspaces(Path::new("/repo"), &[workspace]).is_none());
    }

    #[test]
    fn current_user_identities_empty_when_git_config_is_unavailable() {
        assert!(current_user_identities(Path::new("/repo")).is_empty());
    }
}
