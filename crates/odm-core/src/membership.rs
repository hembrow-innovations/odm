//! Workspace membership — add/rm Project or Progen with kind-specific hooks.

use std::fs;
use std::path::Path;

use odm_git::Git;

use crate::checkout::{materialize, ManagedEntity, MaterializeOutcome};
use crate::config::{
    require_entity_name, save_config, CheckoutMode, ProjectEntry, ProgenEntry, WorkspaceConfig,
};
use crate::error::OdmError;
use crate::gitignore::apply_managed_gitignore;
use crate::gitmodules::rewrite_gitmodules;
use crate::paths::{abs_checkout, progen_index_dir, resolve_under_root, PathResolveError};
use crate::pin_maintain::{maintain_pins_after, prune_pin_file_if_present};

/// Kind of Workspace membership entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipKind {
    Project,
    Progen,
}

impl MembershipKind {
    fn label(self) -> &'static str {
        match self {
            MembershipKind::Project => "project",
            MembershipKind::Progen => "progen",
        }
    }
}

/// Entry payload for membership add.
#[derive(Debug, Clone)]
pub enum MembershipEntry {
    Project(ProjectEntry),
    Progen(ProgenEntry),
}

impl MembershipEntry {
    fn kind(&self) -> MembershipKind {
        match self {
            MembershipEntry::Project(_) => MembershipKind::Project,
            MembershipEntry::Progen(_) => MembershipKind::Progen,
        }
    }

    fn path(&self) -> &str {
        match self {
            MembershipEntry::Project(e) => &e.path,
            MembershipEntry::Progen(e) => &e.path,
        }
    }

    fn url(&self) -> Option<&str> {
        match self {
            MembershipEntry::Project(e) => e.url.as_deref(),
            MembershipEntry::Progen(e) => e.url.as_deref(),
        }
    }

    fn branch(&self) -> Option<&str> {
        match self {
            MembershipEntry::Project(e) => e.branch.as_deref(),
            MembershipEntry::Progen(e) => e.branch.as_deref(),
        }
    }

    fn checkout(&self) -> CheckoutMode {
        match self {
            MembershipEntry::Project(e) => e.checkout,
            MembershipEntry::Progen(e) => e.checkout,
        }
    }
}

/// Add a Project or Progen entry; optional materialize; gitignore + pin maintain.
/// Does not scaffold Progen vaults — that composition lives in `odm-progen`.
pub fn membership_add<R: odm_git::CommandRunner>(
    git: &Git<R>,
    root: &Path,
    config: &mut WorkspaceConfig,
    name: &str,
    entry: MembershipEntry,
    no_clone: bool,
) -> Result<Option<MaterializeOutcome>, OdmError> {
    let kind = entry.kind();
    let label = kind.label();
    // Config load uses workspace errors; add is a CLI mutation → usage.
    require_entity_name(label, name).map_err(|e| match e {
        OdmError::Workspace(m) => OdmError::usage(m),
        other => other,
    })?;
    let already = match kind {
        MembershipKind::Project => config.projects.contains_key(name),
        MembershipKind::Progen => config.progens.contains_key(name),
    };
    if already {
        return Err(OdmError::usage(format!("{label} '{name}' already exists")));
    }
    let cross = match kind {
        MembershipKind::Project => config.progens.contains_key(name),
        MembershipKind::Progen => config.projects.contains_key(name),
    };
    if cross {
        let other = match kind {
            MembershipKind::Project => "progen",
            MembershipKind::Progen => "project",
        };
        return Err(OdmError::usage(format!(
            "name '{name}' is already used as a {other}"
        )));
    }
    if entry.path().trim().is_empty() {
        return Err(OdmError::usage(format!("{label} path must not be empty")));
    }
    resolve_under_root(root, entry.path()).map_err(|e| match e {
        PathResolveError::Absolute { path } => {
            OdmError::usage(format!("{label} path must be relative, got '{path}'"))
        }
        PathResolveError::Escape { path } => OdmError::usage(format!(
            "{label} path must not escape Workspace root, got '{path}'"
        )),
    })?;

    let managed = entry.url().map(|url| ManagedEntity {
        name: name.to_string(),
        path: entry.path().to_string(),
        url: url.to_string(),
        branch: entry.branch().map(|s| s.to_string()),
        checkout: entry.checkout(),
    });

    match entry {
        MembershipEntry::Project(e) => {
            config.projects.insert(name.to_string(), e);
        }
        MembershipEntry::Progen(e) => {
            config.progens.insert(name.to_string(), e);
        }
    }
    save_config(root, config)?;

    if config.manage_gitignore() {
        apply_managed_gitignore(root, config)?;
    }

    let mut outcome = None;
    if let Some(entity) = &managed {
        if !no_clone {
            outcome = Some(materialize(git, root, entity)?);
            if entity.checkout != CheckoutMode::Gitlink {
                maintain_pins_after(git, root, config, &[entity])?;
            }
        }
        if entity.checkout == CheckoutMode::Gitlink {
            rewrite_gitmodules(root, config)?;
        }
    }
    Ok(outcome)
}

/// Remove a Project or Progen from config; optional tree delete.
/// Progen hooks: drop ODM index dir and strip progen_groups members.
pub fn membership_rm<R: odm_git::CommandRunner>(
    git: &Git<R>,
    root: &Path,
    config: &mut WorkspaceConfig,
    kind: MembershipKind,
    name: &str,
    delete: bool,
    force: bool,
) -> Result<(), OdmError> {
    match kind {
        MembershipKind::Project => {
            let entry = config.projects.remove(name).ok_or_else(|| {
                OdmError::usage(format!("unknown project '{name}'"))
            })?;
            if delete {
                maybe_delete_checkout(git, root, name, &entry.path, force, || {
                    config.projects.insert(name.to_string(), entry.clone());
                })?;
            }
        }
        MembershipKind::Progen => {
            let entry = config.progens.remove(name).ok_or_else(|| {
                OdmError::usage(format!("unknown progen '{name}'"))
            })?;
            if delete {
                maybe_delete_checkout(git, root, name, &entry.path, force, || {
                    config.progens.insert(name.to_string(), entry.clone());
                })?;
            }
        }
    }

    if kind == MembershipKind::Progen {
        let idx = progen_index_dir(root, name);
        if idx.exists() {
            let _ = remove_path(&idx);
        }
        for members in config.progen_groups.values_mut() {
            members.retain(|m| m != name);
        }
    }

    save_config(root, config)?;

    if config.manage_gitignore() {
        apply_managed_gitignore(root, config)?;
    }

    prune_pin_file_if_present(root, config)?;
    Ok(())
}

fn maybe_delete_checkout<R, F>(
    git: &Git<R>,
    root: &Path,
    name: &str,
    rel: &str,
    force: bool,
    restore: F,
) -> Result<(), OdmError>
where
    R: odm_git::CommandRunner,
    F: FnOnce(),
{
    let path = abs_checkout(root, rel)?;
    if path.exists() {
        // Own-repo dirty only — nested path-only trees must not inherit ancestor dirt.
        if git.is_repo_root(&path)? && !force && !git.is_clean(&path)? {
            restore();
            return Err(OdmError::operation(format!(
                "working tree dirty for '{name}' (use --force with --delete)"
            )));
        }
        remove_path(&path)?;
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<(), OdmError> {
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|e| {
            OdmError::operation(format!("failed to delete {}: {e}", path.display()))
        })?;
    } else {
        fs::remove_file(path).map_err(|e| {
            OdmError::operation(format!("failed to delete {}: {e}", path.display()))
        })?;
    }
    Ok(())
}

/// Add a project entry; optional materialize; gitignore + pin maintain.
pub fn project_add<R: odm_git::CommandRunner>(
    git: &Git<R>,
    root: &Path,
    config: &mut WorkspaceConfig,
    name: &str,
    entry: ProjectEntry,
    no_clone: bool,
) -> Result<Option<MaterializeOutcome>, OdmError> {
    membership_add(
        git,
        root,
        config,
        name,
        MembershipEntry::Project(entry),
        no_clone,
    )
}

/// Remove project from config; optional tree delete.
pub fn project_rm<R: odm_git::CommandRunner>(
    git: &Git<R>,
    root: &Path,
    config: &mut WorkspaceConfig,
    name: &str,
    delete: bool,
    force: bool,
) -> Result<(), OdmError> {
    membership_rm(git, root, config, MembershipKind::Project, name, delete, force)
}

/// Add a progen entry; optional materialize; gitignore + pin maintain.
/// Vault scaffold is composed by `odm-progen`, not here.
pub fn progen_add<R: odm_git::CommandRunner>(
    git: &Git<R>,
    root: &Path,
    config: &mut WorkspaceConfig,
    name: &str,
    entry: ProgenEntry,
    no_clone: bool,
) -> Result<Option<MaterializeOutcome>, OdmError> {
    membership_add(
        git,
        root,
        config,
        name,
        MembershipEntry::Progen(entry),
        no_clone,
    )
}

/// Remove progen from config; optional tree delete; drop ODM index + group members.
pub fn progen_rm<R: odm_git::CommandRunner>(
    git: &Git<R>,
    root: &Path,
    config: &mut WorkspaceConfig,
    name: &str,
    delete: bool,
    force: bool,
) -> Result<(), OdmError> {
    membership_rm(git, root, config, MembershipKind::Progen, name, delete, force)
}

#[cfg(test)]
#[path = "membership_tests.rs"]
mod tests;
