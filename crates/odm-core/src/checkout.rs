//! Managed checkout — materialize, sync ordering, depth-sort, clone URL resolve.

use std::fs;
use std::path::Path;

use odm_git::Git;

use crate::config::{CheckoutMode, WorkspaceConfig};
use crate::error::OdmError;
use crate::paths::abs_checkout;
use crate::pin_maintain::maintain_pins_after;
use crate::url_match::urls_match_with_root;

/// A managed (url-bearing) Project or Progen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedEntity {
    pub name: String,
    pub path: String,
    pub url: String,
    pub branch: Option<String>,
    pub checkout: CheckoutMode,
}

impl Default for ManagedEntity {
    fn default() -> Self {
        Self {
            name: String::new(),
            path: String::new(),
            url: String::new(),
            branch: None,
            checkout: CheckoutMode::Clone,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterializeOutcome {
    Cloned,
    AlreadyPresent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncResult {
    pub name: String,
    pub materialized: MaterializeOutcome,
    pub fetched: bool,
    pub head: Option<String>,
}

/// Collect managed entities from config (projects + progens with url).
pub fn all_managed(config: &WorkspaceConfig) -> Vec<ManagedEntity> {
    let mut out = Vec::new();
    for (name, e) in &config.projects {
        if let Some(url) = &e.url {
            out.push(ManagedEntity {
                name: name.clone(),
                path: e.path.clone(),
                url: url.clone(),
                branch: e.branch.clone(),
                checkout: e.checkout,
            });
        }
    }
    for (name, e) in &config.progens {
        if let Some(url) = &e.url {
            out.push(ManagedEntity {
                name: name.clone(),
                path: e.path.clone(),
                url: url.clone(),
                branch: e.branch.clone(),
                checkout: e.checkout,
            });
        }
    }
    out
}

/// Resolve named entities; unknown name → Usage.
pub fn resolve_managed(
    config: &WorkspaceConfig,
    names: &[String],
) -> Result<Vec<ManagedEntity>, OdmError> {
    if names.is_empty() {
        return Ok(all_managed(config));
    }
    let mut out = Vec::new();
    for name in names {
        if let Some(e) = config.projects.get(name) {
            let url = e.url.as_ref().ok_or_else(|| {
                OdmError::usage(format!("project '{name}' is path-only (not managed)"))
            })?;
            out.push(ManagedEntity {
                name: name.clone(),
                path: e.path.clone(),
                url: url.clone(),
                branch: e.branch.clone(),
                checkout: e.checkout,
            });
            continue;
        }
        if let Some(e) = config.progens.get(name) {
            let url = e.url.as_ref().ok_or_else(|| {
                OdmError::usage(format!("progen '{name}' is path-only (not managed)"))
            })?;
            out.push(ManagedEntity {
                name: name.clone(),
                path: e.path.clone(),
                url: url.clone(),
                branch: e.branch.clone(),
                checkout: e.checkout,
            });
            continue;
        }
        return Err(OdmError::usage(format!("unknown entity '{name}'")));
    }
    Ok(out)
}

/// Sort managed entries by increasing path depth (parents before children).
pub fn sort_by_depth(entities: &mut [ManagedEntity]) {
    entities.sort_by(|a, b| {
        let da = path_depth(&a.path);
        let db = path_depth(&b.path);
        da.cmp(&db)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.name.cmp(&b.name))
    });
}

fn path_depth(rel: &str) -> usize {
    Path::new(rel)
        .components()
        .filter(|c| !matches!(c, std::path::Component::CurDir))
        .count()
}

/// Resolve config url for `git clone` (relative → absolute under workspace root).
pub fn resolve_clone_url(root: &Path, url: &str) -> String {
    let t = url.trim();
    if t.contains("://") {
        return t.to_string();
    }
    // SCP-like user@host:path
    if let Some(colon) = t.find(':') {
        let left = &t[..colon];
        if left.contains('@') && !Path::new(t).is_absolute() {
            return t.to_string();
        }
    }
    if Path::new(t).is_absolute() {
        return t.to_string();
    }
    root.join(t.trim_start_matches("./"))
        .to_string_lossy()
        .into_owned()
}

/// Ensure managed entry exists as a plain clone with matching origin.
pub fn materialize<R: odm_git::CommandRunner>(
    git: &Git<R>,
    root: &Path,
    entity: &ManagedEntity,
) -> Result<MaterializeOutcome, OdmError> {
    let path = abs_checkout(root, &entity.path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            OdmError::operation(format!(
                "failed to create parent for {}: {e}",
                path.display()
            ))
        })?;
    }

    if !path.exists() {
        let url = resolve_clone_url(root, &entity.url);
        git.clone(&url, &path, entity.branch.as_deref())?;
        return Ok(MaterializeOutcome::Cloned);
    }

    if path.is_file() {
        return Err(OdmError::operation(format!(
            "path exists and is not a directory: {}",
            path.display()
        )));
    }

    // directory
    if is_empty_dir(&path)? {
        let url = resolve_clone_url(root, &entity.url);
        git.clone(&url, &path, entity.branch.as_deref())?;
        return Ok(MaterializeOutcome::Cloned);
    }

    if !git.is_repo(&path)? {
        return Err(OdmError::operation(format!(
            "path exists and is not a git repository: {}",
            path.display()
        )));
    }

    let origin = match git.origin_url(&path) {
        Ok(u) => u,
        Err(odm_git::GitError::OriginMissing { .. }) => {
            return Err(OdmError::operation(format!(
                "origin mismatch for '{}': remote 'origin' is not configured at {}",
                entity.name,
                path.display()
            )));
        }
        Err(e) => return Err(e.into()),
    };

    if !urls_match_with_root(&entity.url, &origin, Some(root)) {
        return Err(OdmError::operation(format!(
            "origin mismatch for '{}': config url '{}' does not match origin '{}'",
            entity.name, entity.url, origin
        )));
    }

    Ok(MaterializeOutcome::AlreadyPresent)
}

fn is_empty_dir(path: &Path) -> Result<bool, OdmError> {
    let mut rd = fs::read_dir(path)
        .map_err(|e| OdmError::operation(format!("failed to read {}: {e}", path.display())))?;
    Ok(rd.next().is_none())
}

/// Materialize if needed, then fetch. Depth-ordered, fail-fast.
/// Pin auto-maintain runs for successful entries when workspace is a git repo.
pub fn sync_managed<R: odm_git::CommandRunner>(
    git: &Git<R>,
    root: &Path,
    config: &WorkspaceConfig,
    names: &[String],
) -> Result<Vec<SyncResult>, OdmError> {
    let mut entities = resolve_managed(config, names)?;
    sort_by_depth(&mut entities);

    let mut results = Vec::new();
    let mut succeeded: Vec<&ManagedEntity> = Vec::new();

    for entity in &entities {
        let outcome = materialize(git, root, entity)?;
        let path = abs_checkout(root, &entity.path)?;
        git.fetch(&path)?;
        let head = git.head_sha(&path).ok();
        results.push(SyncResult {
            name: entity.name.clone(),
            materialized: outcome,
            fetched: true,
            head: head.clone(),
        });
        if head.is_some() {
            succeeded.push(entity);
        }
    }

    maintain_pins_after(git, root, config, &succeeded)?;
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{save_config, ProjectEntry, WorkspaceConfig};
    use crate::init::{init_workspace, InitOptions};
    use crate::pin::load_pin;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::tempdir;

    fn git_user(repo: &Path) {
        Command::new("git")
            .args([
                "-C",
                repo.to_str().unwrap(),
                "config",
                "user.email",
                "t@est",
            ])
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
    fn depth_order_shallow_first() {
        let mut ents = vec![
            ManagedEntity {
                name: "deep".into(),
                path: "a/b/c".into(),
                url: "u".into(),
                branch: None,
                ..Default::default()
            },
            ManagedEntity {
                name: "rootish".into(),
                path: "a".into(),
                url: "u".into(),
                branch: None,
                ..Default::default()
            },
            ManagedEntity {
                name: "mid".into(),
                path: "a/b".into(),
                url: "u".into(),
                branch: None,
                ..Default::default()
            },
        ];
        sort_by_depth(&mut ents);
        assert_eq!(
            ents.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["rootish", "mid", "deep"]
        );
    }

    #[test]
    fn materialize_clone_and_already_present() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let bare = bare_fixture(root, "alpha");
        let g = Git::new();
        let entity = ManagedEntity {
            name: "alpha".into(),
            path: "projects/alpha".into(),
            url: bare.to_string_lossy().into(),
            branch: Some("main".into()),
            ..Default::default()
        };
        assert_eq!(
            materialize(&g, root, &entity).unwrap(),
            MaterializeOutcome::Cloned
        );
        assert_eq!(
            materialize(&g, root, &entity).unwrap(),
            MaterializeOutcome::AlreadyPresent
        );
    }

    #[test]
    fn materialize_origin_mismatch() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let bare_a = bare_fixture(root, "a");
        let bare_b = bare_fixture(root, "b");
        let g = Git::new();
        let path = root.join("proj");
        g.clone(bare_a.to_str().unwrap(), &path, Some("main"))
            .unwrap();
        let entity = ManagedEntity {
            name: "x".into(),
            path: "proj".into(),
            url: bare_b.to_string_lossy().into(),
            branch: None,
            ..Default::default()
        };
        let err = materialize(&g, root, &entity).unwrap_err();
        assert!(err.to_string().contains("origin mismatch"));
        assert!(matches!(err, OdmError::Operation(_)));
    }

    #[test]
    fn materialize_not_git_fails() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let p = root.join("proj");
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("file"), "x").unwrap();
        let g = Git::new();
        let entity = ManagedEntity {
            name: "x".into(),
            path: "proj".into(),
            url: "https://example.com/x.git".into(),
            branch: None,
            ..Default::default()
        };
        let err = materialize(&g, root, &entity).unwrap_err();
        assert!(err.to_string().contains("not a git repository"));
    }

    #[test]
    fn pin_maintain_on_sync_when_ws_git() {
        let dir = tempdir().unwrap();
        let res = init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: false,
            name: Some("t".into()),
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
        let pin = load_pin(&root).unwrap().expect("pin file");
        assert!(pin.pins.contains_key("alpha"));
        assert_eq!(pin.pins["alpha"].rev.len(), 40);
    }

    #[test]
    fn pin_maintain_skipped_when_ws_not_git() {
        let dir = tempdir().unwrap();
        let res = init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: true,
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
        assert!(load_pin(&root).unwrap().is_none());
    }
}
