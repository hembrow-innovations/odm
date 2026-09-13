//! Project git passthrough — run git in primary checkout or worktree slot.

use odm_git::Git;

use crate::checkout::ManagedEntity;
use crate::config::{CheckoutMode, Workspace};
use crate::error::OdmError;
use crate::paths::{abs_checkout, worktree_slot_path};
use crate::pin_maintain::maintain_pins_after;
use crate::worktree::validate_slot_name;

/// Run git passthrough in project checkout (or worktree slot when `wt` is set).
/// Auto-maintain pin if HEAD changed on Primary only (`wt` is `None`).
pub fn project_git<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
    name: &str,
    git_args: &[String],
    wt: Option<&str>,
) -> Result<std::process::ExitStatus, OdmError> {
    if git_args.is_empty() {
        return Err(OdmError::usage(
            "project git requires arguments after --",
        ));
    }
    let entry = ws.config.projects.get(name).ok_or_else(|| {
        OdmError::usage(format!("unknown project '{name}'"))
    })?;

    if let Some(slot) = wt {
        let slot = validate_slot_name(slot)?;
        let path = worktree_slot_path(&ws.root, name, &slot);
        let rel = format!("worktrees/{name}/{slot}");
        if !path.exists() {
            return Err(OdmError::not_found(format!(
                "worktree slot not found: {rel}"
            )));
        }
        if !git.is_repo(&path)? {
            return Err(OdmError::operation(format!(
                "worktree slot path is not a git repo: {rel}"
            )));
        }
        // Pin auto-maintain is Primary-only.
        return Ok(git.run(&path, git_args)?);
    }

    let path = abs_checkout(&ws.root, &entry.path)?;
    if !path.exists() {
        return Err(OdmError::not_found(format!(
            "project path missing: {}",
            entry.path
        )));
    }
    if !git.is_repo(&path)? {
        return Err(OdmError::operation(format!(
            "project path is not a git repo: {}",
            entry.path
        )));
    }

    let before = git.head_sha(&path).ok();
    let status = git.run(&path, git_args)?;
    if status.success() && entry.checkout != CheckoutMode::Gitlink {
        if let Some(url) = &entry.url {
            let after = git.head_sha(&path).ok();
            if after.is_some() && after != before {
                let entity = ManagedEntity {
                    name: name.to_string(),
                    path: entry.path.clone(),
                    url: url.clone(),
                    branch: entry.branch.clone(),
                    checkout: entry.checkout,
                };
                maintain_pins_after(git, &ws.root, &ws.config, &[&entity])?;
            }
        }
    }
    Ok(status)
}

#[cfg(test)]
#[path = "project_git_tests.rs"]
mod tests;
