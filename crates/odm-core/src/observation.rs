//! Workspace observation — sample each declared entity once; derive `PinState`.

use std::path::{Path, PathBuf};

use odm_git::{Git, GitlinkRecord};

use crate::config::{CheckoutMode, WorkspaceConfig};
use crate::error::OdmError;
use crate::paths::resolve_under_root;
use crate::pin::PinFile;
use crate::status::{compute_pin_state, pin_source, PinState};

/// Full Workspace observation snapshot (projects + progens).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceObservation {
    pub root: String,
    pub pin_present: bool,
    pub projects: Vec<EntityObservation>,
    pub progens: Vec<EntityObservation>,
}

impl WorkspaceObservation {
    /// Find an entity row by name (projects first, then progens).
    pub fn find(&self, name: &str) -> Option<&EntityObservation> {
        self.projects
            .iter()
            .chain(self.progens.iter())
            .find(|e| e.name == name)
    }
}

/// Per-entity disk/git/pin facts from one sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityObservation {
    pub name: String,
    pub path: String,
    pub url: Option<String>,
    pub managed: bool,
    /// Absolute path when `resolve_under_root` succeeded.
    pub abs_path: Option<PathBuf>,
    /// Error message when path resolve failed (escape / absolute).
    pub resolve_error: Option<String>,
    pub on_disk: bool,
    pub is_git: bool,
    pub head: Option<String>,
    pub origin: Option<String>,
    pub dirty: Option<bool>,
    pub pin_rev: Option<String>,
    pub pin_state: PinState,
}

/// Sample every declared Project and Progen once. Does not fetch.
pub fn observe_workspace<R: odm_git::CommandRunner>(
    git: &Git<R>,
    root: &Path,
    config: &WorkspaceConfig,
    pin: Option<&PinFile>,
) -> Result<WorkspaceObservation, OdmError> {
    let pin_present = pin.is_some();
    let mut projects = Vec::with_capacity(config.projects.len());
    for (name, entry) in &config.projects {
        projects.push(observe_entity(
            git,
            root,
            name,
            &entry.path,
            entry.url.as_deref(),
            entry.is_managed(),
            entry.checkout,
            pin,
        )?);
    }
    let mut progens = Vec::with_capacity(config.progens.len());
    for (name, entry) in &config.progens {
        progens.push(observe_entity(
            git,
            root,
            name,
            &entry.path,
            entry.url.as_deref(),
            entry.is_managed(),
            entry.checkout,
            pin,
        )?);
    }
    Ok(WorkspaceObservation {
        root: root.display().to_string(),
        pin_present,
        projects,
        progens,
    })
}

/// Sample one entity (path policy: `resolve_under_root`).
pub fn observe_entity<R: odm_git::CommandRunner>(
    git: &Git<R>,
    root: &Path,
    name: &str,
    rel_path: &str,
    url: Option<&str>,
    managed: bool,
    checkout: CheckoutMode,
    pin: Option<&PinFile>,
) -> Result<EntityObservation, OdmError> {
    let pin_present = pin.is_some();
    let pin_rev = if checkout == CheckoutMode::Gitlink {
        match git.gitlink_record(root, Path::new(rel_path)).ok() {
            Some(GitlinkRecord::Recorded { sha }) => Some(sha),
            _ => None,
        }
    } else {
        pin.and_then(|p| p.pins.get(name)).map(|e| e.rev.clone())
    };

    let (abs_path, resolve_error, on_disk, is_git, head, origin, dirty) =
        match resolve_under_root(root, rel_path) {
            Ok(abs) => {
                let on_disk = abs.exists();
                // Own checkout only — not nested inside an ancestor Workspace/monorepo git.
                let is_git = if on_disk {
                    git.is_repo_root(&abs).unwrap_or(false)
                } else {
                    false
                };
                let (head, origin, dirty) = if is_git {
                    (
                        git.head_sha(&abs).ok(),
                        git.origin_url(&abs).ok(),
                        git.is_clean(&abs).ok().map(|c| !c),
                    )
                } else {
                    (None, None, None)
                };
                (Some(abs), None, on_disk, is_git, head, origin, dirty)
            }
            Err(e) => (None, Some(e.to_string()), false, false, None, None, None),
        };

    let pin_state = compute_pin_state(
        pin_source(managed, checkout),
        pin_present,
        on_disk,
        pin_rev.as_deref(),
        head.as_deref(),
    );

    Ok(EntityObservation {
        name: name.into(),
        path: rel_path.into(),
        url: url.map(str::to_string),
        managed,
        abs_path,
        resolve_error,
        on_disk,
        is_git,
        head,
        origin,
        dirty,
        pin_rev,
        pin_state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{save_config, ProjectEntry, WorkspaceConfig};
    use crate::init::{init_workspace, InitOptions};
    use crate::pin::{save_pin, PinEntry, PinFile};
    use crate::status::PinState;
    use odm_git::Git;
    use tempfile::tempdir;

    #[test]
    fn pin_source_gitlink_observe_skips_missing_pin_file() {
        let dir = tempdir().unwrap();
        init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: true,
            name: None,
        })
        .unwrap();
        let mut cfg = WorkspaceConfig::default();
        cfg.projects.insert(
            "link".into(),
            ProjectEntry {
                path: "vendor/link".into(),
                url: Some("https://example.com/l.git".into()),
                checkout: crate::config::CheckoutMode::Gitlink,
                ..Default::default()
            },
        );
        save_config(dir.path(), &cfg).unwrap();
        let git = Git::new();
        let obs = observe_workspace(&git, dir.path(), &cfg, None).unwrap();
        let link = obs.find("link").unwrap();
        assert_ne!(link.pin_state, PinState::MissingPinFile);
        assert_eq!(link.pin_state, PinState::Unpinned);
    }

    #[test]
    fn observe_attaches_pin_state_via_classifier() {
        let dir = tempdir().unwrap();
        init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: true,
            name: None,
        })
        .unwrap();
        let mut cfg = WorkspaceConfig::default();
        cfg.projects.insert(
            "local".into(),
            ProjectEntry {
                path: "projects/local".into(),
                url: None,
                branch: None,
                type_: None,
                ..Default::default()
            },
        );
        cfg.projects.insert(
            "managed".into(),
            ProjectEntry {
                path: "projects/managed".into(),
                url: Some("https://example.com/m.git".into()),
                branch: None,
                type_: None,
                ..Default::default()
            },
        );
        save_config(dir.path(), &cfg).unwrap();

        let git = Git::new();
        // no pin file
        let obs = observe_workspace(&git, dir.path(), &cfg, None).unwrap();
        let local = obs.find("local").unwrap();
        assert_eq!(local.pin_state, PinState::None);
        assert!(!local.on_disk);
        let managed = obs.find("managed").unwrap();
        assert_eq!(managed.pin_state, PinState::MissingPinFile);

        // pin present, no entries → unpinned (even when path missing)
        let pin = PinFile::new_v1();
        save_pin(dir.path(), &pin).unwrap();
        let obs = observe_workspace(&git, dir.path(), &cfg, Some(&pin)).unwrap();
        assert_eq!(obs.find("managed").unwrap().pin_state, PinState::Unpinned);

        // pin entry, path missing → missing_path
        let mut pin = PinFile::new_v1();
        pin.pins.insert(
            "managed".into(),
            PinEntry {
                rev: "a".repeat(40),
                url: "https://example.com/m.git".into(),
                branch: None,
            },
        );
        let obs = observe_workspace(&git, dir.path(), &cfg, Some(&pin)).unwrap();
        assert_eq!(obs.find("managed").unwrap().pin_state, PinState::MissingPath);

        // path on disk but not git + pin_rev → drift
        std::fs::create_dir_all(dir.path().join("projects/managed")).unwrap();
        let obs = observe_workspace(&git, dir.path(), &cfg, Some(&pin)).unwrap();
        let m = obs.find("managed").unwrap();
        assert!(m.on_disk);
        assert!(!m.is_git);
        assert_eq!(m.pin_state, PinState::Drift);
    }

    #[test]
    fn observe_path_escape_records_resolve_error() {
        let dir = tempdir().unwrap();
        init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: true,
            name: None,
        })
        .unwrap();
        let mut cfg = WorkspaceConfig::default();
        cfg.projects.insert(
            "bad".into(),
            ProjectEntry {
                path: "../outside".into(),
                url: None,
                branch: None,
                type_: None,
                ..Default::default()
            },
        );
        save_config(dir.path(), &cfg).unwrap();
        let git = Git::new();
        let obs = observe_workspace(&git, dir.path(), &cfg, None).unwrap();
        let bad = obs.find("bad").unwrap();
        assert!(bad.resolve_error.is_some());
        assert!(!bad.on_disk);
        assert_eq!(bad.pin_state, PinState::None);
    }

    #[test]
    fn path_only_under_workspace_git_is_not_is_git() {
        let dir = tempdir().unwrap();
        init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: false,
            name: None,
        })
        .unwrap();
        let mut cfg = WorkspaceConfig::default();
        cfg.progens.insert(
            "desk".into(),
            crate::config::ProgenEntry {
                path: "progens/desk".into(),
                url: None,
                branch: None,
                ..Default::default()
            },
        );
        save_config(dir.path(), &cfg).unwrap();
        std::fs::create_dir_all(dir.path().join("progens/desk")).unwrap();
        std::fs::write(dir.path().join("progens/desk/Note.md"), "# n\n").unwrap();
        // Dirty the Workspace git root — must not leak onto path-only progen.
        std::fs::write(dir.path().join("stray.txt"), "x").unwrap();

        let git = Git::new();
        let obs = observe_workspace(&git, dir.path(), &cfg, None).unwrap();
        let desk = obs.find("desk").unwrap();
        assert!(desk.on_disk);
        assert!(!desk.is_git, "path-only must not inherit ancestor git");
        assert_eq!(desk.dirty, None);
        assert!(desk.head.is_none());
    }

    #[test]
    fn own_git_at_entity_path_is_is_git() {
        let dir = tempdir().unwrap();
        init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: false,
            name: None,
        })
        .unwrap();
        let mut cfg = WorkspaceConfig::default();
        cfg.progens.insert(
            "vault".into(),
            crate::config::ProgenEntry {
                path: "progens/vault".into(),
                url: None,
                branch: None,
                ..Default::default()
            },
        );
        save_config(dir.path(), &cfg).unwrap();
        let vault = dir.path().join("progens/vault");
        std::fs::create_dir_all(&vault).unwrap();
        let git = Git::new();
        git.init(&vault).unwrap();
        std::fs::write(vault.join("a.md"), "x").unwrap();

        let obs = observe_workspace(&git, dir.path(), &cfg, None).unwrap();
        let v = obs.find("vault").unwrap();
        assert!(v.is_git);
        assert_eq!(v.dirty, Some(true));
    }
}
