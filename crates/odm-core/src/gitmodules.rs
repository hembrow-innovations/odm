//! `.gitmodules` is rewritten from config gitlink entries. Config is layout truth.

use std::fs;
use std::path::Path;

use crate::checkout::all_managed;
use crate::config::{CheckoutMode, WorkspaceConfig};
use crate::error::OdmError;
use crate::io::atomic_write;

pub fn gitmodules_path(root: &Path) -> std::path::PathBuf {
    root.join(".gitmodules")
}

/// Desired `.gitmodules` body from config gitlink entries. `None` means the file should not exist.
pub fn desired_gitmodules(config: &WorkspaceConfig) -> Option<String> {
    let mut entries: Vec<_> = all_managed(config)
        .into_iter()
        .filter(|e| e.checkout == CheckoutMode::Gitlink)
        .collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.name.cmp(&b.name)));
    if entries.is_empty() {
        return None;
    }
    let mut body = String::new();
    for e in &entries {
        body.push_str(&format!("[submodule \"{}\"]\n", e.path));
        body.push_str(&format!("\tpath = {}\n", e.path));
        body.push_str(&format!("\turl = {}\n", e.url));
        if let Some(branch) = &e.branch {
            body.push_str(&format!("\tbranch = {branch}\n"));
        }
    }
    Some(body)
}

pub fn gitmodules_has_drift(root: &Path, config: &WorkspaceConfig) -> bool {
    let path = gitmodules_path(root);
    match desired_gitmodules(config) {
        None => path.exists(),
        Some(desired) => fs::read_to_string(&path).ok().as_deref() != Some(desired.as_str()),
    }
}

/// Write `.gitmodules` from config gitlink entries. Removes the file when none.
pub fn rewrite_gitmodules(root: &Path, config: &WorkspaceConfig) -> Result<(), OdmError> {
    let path = gitmodules_path(root);
    match desired_gitmodules(config) {
        None => {
            if path.exists() {
                fs::remove_file(&path).map_err(|e| {
                    OdmError::operation(format!("failed to remove {}: {e}", path.display()))
                })?;
            }
            Ok(())
        }
        Some(body) => atomic_write(&path, &body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ProgenEntry, ProjectEntry};

    #[test]
    fn materialize_gitlink_rewrite_gitmodules_from_config() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut cfg = WorkspaceConfig::default();
        cfg.projects.insert(
            "clone".into(),
            ProjectEntry {
                path: "projects/clone".into(),
                url: Some("https://example.com/clone.git".into()),
                branch: Some("main".into()),
                ..Default::default()
            },
        );
        cfg.projects.insert(
            "nested".into(),
            ProjectEntry {
                path: "vendor/nested".into(),
                url: Some("https://example.com/nested.git".into()),
                branch: Some("main".into()),
                checkout: CheckoutMode::Gitlink,
                ..Default::default()
            },
        );
        cfg.progens.insert(
            "docs".into(),
            ProgenEntry {
                path: "vendor/docs".into(),
                url: Some("https://example.com/docs.git".into()),
                checkout: CheckoutMode::Gitlink,
                ..Default::default()
            },
        );
        rewrite_gitmodules(root, &cfg).unwrap();
        let text = fs::read_to_string(gitmodules_path(root)).unwrap();
        assert!(text.contains("[submodule \"vendor/nested\"]"), "{text}");
        assert!(text.contains("path = vendor/nested"), "{text}");
        assert!(
            text.contains("url = https://example.com/nested.git"),
            "{text}"
        );
        assert!(text.contains("branch = main"), "{text}");
        assert!(text.contains("[submodule \"vendor/docs\"]"), "{text}");
        assert!(text.contains("path = vendor/docs"), "{text}");
        assert!(!text.contains("projects/clone"), "{text}");
        let nested_at = text.find("vendor/docs").unwrap();
        let nested_path_at = text.find("vendor/nested").unwrap();
        assert!(nested_at < nested_path_at, "sorted by path: {text}");
    }

    #[test]
    fn materialize_gitlink_rewrite_gitmodules_removes_when_none() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(gitmodules_path(root), "[submodule \"gone\"]\n").unwrap();
        rewrite_gitmodules(root, &WorkspaceConfig::default()).unwrap();
        assert!(!gitmodules_path(root).exists());
    }

    #[test]
    fn gitmodules_layout_drift_then_rewrite_matches() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut cfg = WorkspaceConfig::default();
        cfg.projects.insert(
            "nested".into(),
            ProjectEntry {
                path: "vendor/nested".into(),
                url: Some("https://example.com/nested.git".into()),
                checkout: CheckoutMode::Gitlink,
                ..Default::default()
            },
        );
        assert!(gitmodules_has_drift(root, &cfg));
        fs::write(gitmodules_path(root), "[submodule \"stale\"]\n").unwrap();
        assert!(gitmodules_has_drift(root, &cfg));
        rewrite_gitmodules(root, &cfg).unwrap();
        assert!(!gitmodules_has_drift(root, &cfg));
        assert_eq!(
            fs::read_to_string(gitmodules_path(root)).unwrap(),
            desired_gitmodules(&cfg).unwrap()
        );
    }
}
