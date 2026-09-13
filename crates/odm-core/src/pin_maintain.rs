//! Pin maintenance — auto-maintain after HEAD changes; status and apply.

use std::path::Path;

use odm_git::Git;

use crate::checkout::{all_managed, resolve_managed, ManagedEntity};
use crate::config::WorkspaceConfig;
use crate::error::OdmError;
use crate::paths::abs_checkout;
use crate::pin::{load_pin, prune_pins, save_pin, PinEntry, PinFile};
use crate::status::{pin_source, PinSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyAction {
    Refuse,
    Force,
}

pub(crate) fn lockfile_pin_names(entities: &[ManagedEntity]) -> Vec<&str> {
    entities
        .iter()
        .filter(|e| pin_source(true, e.checkout) == PinSource::LockFile)
        .map(|e| e.name.as_str())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinStatusReport {
    pub pin_file: String,
    pub present: bool,
    pub entries: Vec<PinStatusEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinStatusEntry {
    pub name: String,
    pub pin_rev: Option<String>,
    pub head: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinApplyResult {
    pub name: String,
    pub status: String,
    pub rev: Option<String>,
    /// Always true on success — apply checks out detached HEAD by design.
    pub detached: bool,
}

/// Create/update pin file for managed entities that have a defined HEAD.
/// No-op when workspace root is not a git repo.
pub fn maintain_pins_after<R: odm_git::CommandRunner>(
    git: &Git<R>,
    root: &Path,
    config: &WorkspaceConfig,
    entities: &[&ManagedEntity],
) -> Result<(), OdmError> {
    if !git.is_repo(root)? {
        return Ok(());
    }

    let managed = all_managed(config);
    let managed_names = lockfile_pin_names(&managed);

    let mut pin = match load_pin(root)? {
        Some(p) => p,
        None => {
            if entities.is_empty() {
                return Ok(());
            }
            PinFile::new_v1()
        }
    };

    prune_pins(&mut pin, &managed_names);

    for entity in entities {
        if pin_source(true, entity.checkout) != PinSource::LockFile {
            continue;
        }
        let path = abs_checkout(root, &entity.path)?;
        if !path.exists() || !git.is_repo(&path)? {
            continue;
        }
        let rev = match git.head_sha(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        pin.pins.insert(
            entity.name.clone(),
            PinEntry {
                rev,
                url: entity.url.clone(),
                branch: entity.branch.clone(),
            },
        );
    }

    if pin.pins.is_empty() && !pin_path_exists(root) {
        return Ok(());
    }

    save_pin(root, &pin)
}

fn pin_path_exists(root: &Path) -> bool {
    crate::paths::pin_path(root).is_file()
}

/// Pin status for named entities (or all managed when empty).
pub fn pin_status<R: odm_git::CommandRunner>(
    git: &Git<R>,
    root: &Path,
    config: &WorkspaceConfig,
    names: &[String],
) -> Result<PinStatusReport, OdmError> {
    let pin = load_pin(root)?;
    let present = pin.is_some();
    let pin_file = ".odm/odm.lock.yaml".to_string();
    let obs = crate::observation::observe_workspace(git, root, config, pin.as_ref())?;

    let entities = if names.is_empty() {
        all_managed(config)
    } else {
        resolve_managed(config, names)?
    };

    let mut entries = Vec::new();
    for entity in entities {
        let row = obs.find(&entity.name).ok_or_else(|| {
            OdmError::usage(format!("unknown entity '{}'", entity.name))
        })?;
        entries.push(PinStatusEntry {
            name: entity.name,
            pin_rev: row.pin_rev.clone(),
            head: row.head.clone(),
            state: row.pin_state.as_str().to_string(),
        });
    }

    Ok(PinStatusReport {
        pin_file,
        present,
        entries,
    })
}

/// Apply pins (detached HEAD). Dirty → fail unless force. Missing path → NotFound.
pub fn pin_apply<R: odm_git::CommandRunner>(
    git: &Git<R>,
    root: &Path,
    config: &WorkspaceConfig,
    names: &[String],
    force: bool,
) -> Result<Vec<PinApplyResult>, OdmError> {
    let pin = load_pin(root)?.ok_or_else(|| {
        OdmError::not_found("pin file not found: .odm/odm.lock.yaml")
    })?;

    let targets: Vec<(String, PinEntry, String)> = if names.is_empty() {
        let mut v = Vec::new();
        for (name, entry) in &pin.pins {
            let path = find_managed_path(config, name).ok_or_else(|| {
                OdmError::usage(format!(
                    "pin '{name}' has no managed config entry"
                ))
            })?;
            v.push((name.clone(), entry.clone(), path));
        }
        v
    } else {
        let mut v = Vec::new();
        for name in names {
            let entry = pin.pins.get(name).ok_or_else(|| {
                OdmError::not_found(format!("no pin for '{name}'"))
            })?;
            let path = find_managed_path(config, name).ok_or_else(|| {
                OdmError::usage(format!("unknown or unmanaged entity '{name}'"))
            })?;
            v.push((name.clone(), entry.clone(), path));
        }
        v
    };

    let mut results = Vec::new();
    for (name, entry, rel) in targets {
        let path = abs_checkout(root, &rel)?;
        if !path.exists() {
            return Err(OdmError::not_found(format!(
                "path missing for '{name}': {rel}"
            )));
        }
        if !git.is_repo(&path)? {
            return Err(OdmError::not_found(format!(
                "path is not a git repo for '{name}': {rel}"
            )));
        }
        let dirty = if force {
            DirtyAction::Force
        } else {
            DirtyAction::Refuse
        };
        if dirty == DirtyAction::Refuse && !git.is_clean(&path)? {
            return Err(OdmError::operation(format!(
                "working tree dirty for '{name}' (use --force)"
            )));
        }
        git.checkout_detached(&path, &entry.rev)?;
        results.push(PinApplyResult {
            name,
            status: "applied".into(),
            rev: Some(entry.rev.clone()),
            detached: true,
        });
    }
    Ok(results)
}

fn find_managed_path(config: &WorkspaceConfig, name: &str) -> Option<String> {
    if let Some(e) = config.projects.get(name) {
        if e.url.is_some() {
            return Some(e.path.clone());
        }
    }
    if let Some(e) = config.progens.get(name) {
        if e.url.is_some() {
            return Some(e.path.clone());
        }
    }
    None
}

/// Prune pin file to current managed set when a pin file already exists.
pub(crate) fn prune_pin_file_if_present(
    root: &Path,
    config: &WorkspaceConfig,
) -> Result<(), OdmError> {
    if let Some(mut pin) = load_pin(root)? {
        let managed = all_managed(config);
        let names = lockfile_pin_names(&managed);
        prune_pins(&mut pin, &names);
        save_pin(root, &pin)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkout::{sync_managed, ManagedEntity};
    use crate::config::{save_config, ProjectEntry, WorkspaceConfig};
    use crate::init::{init_workspace, InitOptions};
    use crate::pin::load_pin;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::tempdir;

    fn git_user(repo: &Path) {
        Command::new("git")
            .args(["-C", repo.to_str().unwrap(), "config", "user.email", "t@est"])
            .status()
            .unwrap();
        Command::new("git")
            .args(["-C", repo.to_str().unwrap(), "config", "user.name", "t"])
            .status()
            .unwrap();
    }

    fn bare_fixture(root: &Path, name: &str) -> PathBuf {
        let bare = root.join(format!("{name}.git"));
        assert!(Command::new("git")
            .args(["init", "--bare", bare.to_str().unwrap()])
            .status()
            .unwrap()
            .success());
        let seed = root.join(format!("{name}-seed"));
        assert!(Command::new("git")
            .args(["clone", bare.to_str().unwrap(), seed.to_str().unwrap()])
            .status()
            .unwrap()
            .success());
        git_user(&seed);
        fs::write(seed.join("README"), name).unwrap();
        assert!(Command::new("git")
            .args(["-C", seed.to_str().unwrap(), "add", "README"])
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["-C", seed.to_str().unwrap(), "commit", "-m", "init"])
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["-C", seed.to_str().unwrap(), "branch", "-M", "main"])
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["-C", seed.to_str().unwrap(), "push", "-u", "origin", "main"])
            .status()
            .unwrap()
            .success());
        bare
    }

    #[test]
    fn pin_source_lockfile_names_skip_gitlink() {
        use crate::config::CheckoutMode;
        let ents = vec![
            ManagedEntity {
                name: "clone".into(),
                path: "a".into(),
                url: "u".into(),
                checkout: CheckoutMode::Clone,
                ..Default::default()
            },
            ManagedEntity {
                name: "link".into(),
                path: "b".into(),
                url: "u".into(),
                checkout: CheckoutMode::Gitlink,
                ..Default::default()
            },
        ];
        assert_eq!(lockfile_pin_names(&ents), vec!["clone"]);
        let _ = DirtyAction::Refuse;
        let _ = DirtyAction::Force;
        assert_ne!(DirtyAction::Refuse, DirtyAction::Force);
    }

    #[test]
    fn pin_apply_and_status() {
        let dir = tempdir().unwrap();
        let res = init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: false,
            name: None,
        })
        .unwrap();
        let root = res.root;
        let bare = bare_fixture(&root, "alpha");
        let mut cfg = WorkspaceConfig::default();
        cfg.projects.insert(
            "alpha".into(),
            ProjectEntry {
                path: "projects/alpha".into(),
                url: Some(bare.to_string_lossy().into()),
                branch: Some("main".into()),
                type_: None,
                ..Default::default()
            },
        );
        save_config(&root, &cfg).unwrap();
        let g = Git::new();
        sync_managed(&g, &root, &cfg, &[]).unwrap();
        let pin = load_pin(&root).unwrap().unwrap();
        let rev = pin.pins["alpha"].rev.clone();

        let st = pin_status(&g, &root, &cfg, &[]).unwrap();
        assert!(st.present);
        assert_eq!(st.entries[0].state, "in_sync");

        let applied = pin_apply(&g, &root, &cfg, &[], false).unwrap();
        assert_eq!(applied[0].status, "applied");
        assert!(applied[0].detached);
        assert_eq!(applied[0].rev.as_deref(), Some(rev.as_str()));
    }

    #[test]
    fn pin_apply_dirty_fails_without_force() {
        let dir = tempdir().unwrap();
        let res = init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: false,
            name: None,
        })
        .unwrap();
        let root = res.root;
        let bare = bare_fixture(&root, "alpha");
        let mut cfg = WorkspaceConfig::default();
        cfg.projects.insert(
            "alpha".into(),
            ProjectEntry {
                path: "projects/alpha".into(),
                url: Some(bare.to_string_lossy().into()),
                branch: Some("main".into()),
                type_: None,
                ..Default::default()
            },
        );
        save_config(&root, &cfg).unwrap();
        let g = Git::new();
        sync_managed(&g, &root, &cfg, &[]).unwrap();
        fs::write(root.join("projects/alpha/dirty"), "x").unwrap();
        let err = pin_apply(&g, &root, &cfg, &[], false).unwrap_err();
        assert!(err.to_string().contains("dirty"));
        pin_apply(&g, &root, &cfg, &[], true).unwrap();
    }
}
