use std::fs;
use std::path::Path;

use crate::config::WorkspaceConfig;
use crate::error::OdmError;
use crate::io::atomic_write;

pub const BEGIN_MARKER: &str = "# >>> ODM managed (do not edit between markers)";
pub const END_MARKER: &str = "# <<< ODM managed";

const EPHEMERAL: &[&str] = &[".odm/cache/", ".odm/log/", ".odm/progen/", "worktrees/"];

/// Desired inner lines for the Workspace-root managed block (sorted, unique).
pub fn desired_workspace_lines(config: &WorkspaceConfig) -> Vec<String> {
    let mut lines: Vec<String> = EPHEMERAL.iter().map(|s| (*s).to_string()).collect();
    for entry in config.projects.values() {
        if entry.is_managed() {
            lines.push(with_trailing_slash(&entry.path));
        }
    }
    for entry in config.progens.values() {
        if entry.is_managed() {
            lines.push(with_trailing_slash(&entry.path));
        }
    }
    lines.sort();
    lines.dedup();
    lines
}

/// Desired inner lines for an ancestor managed checkout (children relative to parent).
pub fn desired_ancestor_lines(child_rels: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = child_rels.iter().map(|p| with_trailing_slash(p)).collect();
    lines.sort();
    lines.dedup();
    lines
}

/// Format a full managed block (markers + inner lines + trailing newline).
pub fn desired_block(inner_lines: &[String]) -> String {
    format_block(inner_lines)
}

/// Seed/update Workspace-root `.gitignore` managed block only.
pub fn update_workspace_gitignore(
    root: &Path,
    config: &WorkspaceConfig,
) -> Result<(), OdmError> {
    if !config.manage_gitignore() {
        return Ok(());
    }
    let lines = desired_workspace_lines(config);
    write_managed_block(&root.join(".gitignore"), &lines)
}

/// Rewrite Workspace-root and ancestor-checkout managed gitignore blocks.
///
/// No-op when `manage_gitignore` is false. Does not require Workspace root to be a git
/// repo (caller decides); still only touches ignore files.
pub fn apply_managed_gitignore(
    root: &Path,
    config: &WorkspaceConfig,
) -> Result<(), OdmError> {
    if !config.manage_gitignore() {
        return Ok(());
    }
    update_workspace_gitignore(root, config)?;

    let managed = managed_paths(config);
    for (parent, children) in ancestor_child_groups(&managed) {
        let parent_abs = root.join(&parent);
        if !parent_abs.is_dir() {
            continue;
        }
        let lines = desired_ancestor_lines(&children);
        write_managed_block(&parent_abs.join(".gitignore"), &lines)?;
    }
    Ok(())
}

/// True when on-disk managed block at Workspace root differs from desired (or missing).
/// Returns `false` when manage_gitignore is off (no drift obligation).
pub fn workspace_gitignore_has_drift(
    root: &Path,
    config: &WorkspaceConfig,
) -> Result<bool, OdmError> {
    if !config.manage_gitignore() {
        return Ok(false);
    }
    let path = root.join(".gitignore");
    let desired = desired_workspace_lines(config);
    on_disk_differs(&path, &desired)
}

/// True when any ancestor managed checkout gitignore differs from desired.
pub fn ancestor_gitignore_has_drift(
    root: &Path,
    config: &WorkspaceConfig,
) -> Result<bool, OdmError> {
    if !config.manage_gitignore() {
        return Ok(false);
    }
    let managed = managed_paths(config);
    for (parent, children) in ancestor_child_groups(&managed) {
        let parent_abs = root.join(&parent);
        if !parent_abs.is_dir() {
            continue;
        }
        let desired = desired_ancestor_lines(&children);
        if on_disk_differs(&parent_abs.join(".gitignore"), &desired)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn on_disk_differs(gitignore_path: &Path, desired_inner: &[String]) -> Result<bool, OdmError> {
    let desired = format_block(desired_inner);
    if !gitignore_path.is_file() {
        return Ok(true);
    }
    let existing = fs::read_to_string(gitignore_path).map_err(|e| {
        OdmError::operation(format!(
            "failed to read {}: {e}",
            gitignore_path.display()
        ))
    })?;
    match extract_managed_block(&existing) {
        Some(block) => Ok(normalize_block(&block) != normalize_block(&desired)),
        None => Ok(true),
    }
}

fn normalize_block(s: &str) -> String {
    // Compare without requiring identical trailing whitespace quirks outside markers.
    s.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract the full managed block text including markers (and trailing newline after end).
pub fn extract_managed_block(text: &str) -> Option<String> {
    let begin = text.find(BEGIN_MARKER)?;
    let end_rel = text[begin..].find(END_MARKER)?;
    let end = begin + end_rel + END_MARKER.len();
    let mut end_incl = end;
    if text[end_incl..].starts_with('\n') {
        end_incl += 1;
    }
    Some(text[begin..end_incl].to_string())
}

fn managed_paths(config: &WorkspaceConfig) -> Vec<String> {
    let mut paths = Vec::new();
    for entry in config.projects.values() {
        if entry.is_managed() {
            paths.push(normalize_rel(&entry.path));
        }
    }
    for entry in config.progens.values() {
        if entry.is_managed() {
            paths.push(normalize_rel(&entry.path));
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Groups: parent managed path → child paths relative to parent.
fn ancestor_child_groups(managed: &[String]) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for parent in managed {
        let mut children = Vec::new();
        for child in managed {
            if child == parent {
                continue;
            }
            if let Some(rel) = relative_child(parent, child) {
                children.push(rel);
            }
        }
        if !children.is_empty() {
            groups.push((parent.clone(), children));
        }
    }
    groups
}

/// If `child` is nested under `parent`, return child path relative to parent.
fn relative_child(parent: &str, child: &str) -> Option<String> {
    let p = normalize_rel(parent);
    let c = normalize_rel(child);
    let prefix = format!("{p}/");
    c.strip_prefix(&prefix).map(|s| s.to_string())
}

fn normalize_rel(path: &str) -> String {
    path.trim().trim_end_matches('/').to_string()
}

fn with_trailing_slash(path: &str) -> String {
    let p = path.trim_end_matches('/');
    format!("{p}/")
}

pub fn write_managed_block(gitignore_path: &Path, inner_lines: &[String]) -> Result<(), OdmError> {
    let existing = if gitignore_path.is_file() {
        fs::read_to_string(gitignore_path).map_err(|e| {
            OdmError::operation(format!(
                "failed to read {}: {e}",
                gitignore_path.display()
            ))
        })?
    } else {
        String::new()
    };

    let new_block = format_block(inner_lines);
    let updated = replace_or_append_block(&existing, &new_block);
    if let Some(parent) = gitignore_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| {
                OdmError::operation(format!("failed to create {}: {e}", parent.display()))
            })?;
        }
    }
    atomic_write(gitignore_path, &updated)
}

fn format_block(inner_lines: &[String]) -> String {
    let mut s = String::new();
    s.push_str(BEGIN_MARKER);
    s.push('\n');
    for line in inner_lines {
        s.push_str(line);
        s.push('\n');
    }
    s.push_str(END_MARKER);
    s.push('\n');
    s
}

fn replace_or_append_block(existing: &str, new_block: &str) -> String {
    if let Some((before, after)) = split_markers(existing) {
        let mut out = before;
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(new_block);
        let after = after.trim_start_matches('\n');
        if !after.is_empty() {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(after);
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        return out;
    }

    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(new_block);
    out
}

fn split_markers(text: &str) -> Option<(String, String)> {
    let begin = text.find(BEGIN_MARKER)?;
    let end_rel = text[begin..].find(END_MARKER)?;
    let end = begin + end_rel + END_MARKER.len();
    let mut after_start = end;
    if text[after_start..].starts_with('\n') {
        after_start += 1;
    }
    Some((text[..begin].to_string(), text[after_start..].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectEntry;
    use tempfile::tempdir;

    #[test]
    fn seed_ephemeral_and_managed() {
        let dir = tempdir().unwrap();
        let mut cfg = WorkspaceConfig::default();
        cfg.projects.insert(
            "a".into(),
            ProjectEntry {
                path: "projects/alpha".into(),
                url: Some("u".into()),
                branch: None,
                type_: None,
                ..Default::default()
            },
        );
        cfg.projects.insert(
            "local".into(),
            ProjectEntry {
                path: "local".into(),
                url: None,
                branch: None,
                type_: None,
                ..Default::default()
            },
        );
        update_workspace_gitignore(dir.path(), &cfg).unwrap();
        let text = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(text.contains(BEGIN_MARKER));
        assert!(text.contains(".odm/cache/"));
        assert!(text.contains("projects/alpha/"));
        assert!(!text.contains("local/"));
    }

    #[test]
    fn rewrite_idempotent_preserves_outside() {
        let dir = tempdir().unwrap();
        let gi = dir.path().join(".gitignore");
        fs::write(&gi, "user-line\n").unwrap();
        let cfg = WorkspaceConfig::default();
        update_workspace_gitignore(dir.path(), &cfg).unwrap();
        update_workspace_gitignore(dir.path(), &cfg).unwrap();
        let text = fs::read_to_string(&gi).unwrap();
        assert_eq!(text.matches(BEGIN_MARKER).count(), 1);
        assert!(text.starts_with("user-line\n"));
        let again = fs::read_to_string(&gi).unwrap();
        update_workspace_gitignore(dir.path(), &cfg).unwrap();
        assert_eq!(fs::read_to_string(&gi).unwrap(), again);
    }

    #[test]
    fn disabled_noop() {
        let dir = tempdir().unwrap();
        let cfg = WorkspaceConfig {
            manage_gitignore: Some(false),
            ..Default::default()
        };
        update_workspace_gitignore(dir.path(), &cfg).unwrap();
        assert!(!dir.path().join(".gitignore").exists());
    }

    #[test]
    fn desired_block_stable() {
        let lines = vec![".odm/cache/".into(), "worktrees/".into()];
        let a = desired_block(&lines);
        let b = desired_block(&lines);
        assert_eq!(a, b);
        assert!(a.starts_with(BEGIN_MARKER));
        assert!(a.contains(END_MARKER));
    }

    #[test]
    fn ancestor_nested_gitignore() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let parent = root.join("projects/mono");
        fs::create_dir_all(parent.join("nested")).unwrap();

        let mut cfg = WorkspaceConfig::default();
        cfg.projects.insert(
            "mono".into(),
            ProjectEntry {
                path: "projects/mono".into(),
                url: Some("u1".into()),
                branch: None,
                type_: None,
                ..Default::default()
            },
        );
        cfg.projects.insert(
            "nested".into(),
            ProjectEntry {
                path: "projects/mono/nested".into(),
                url: Some("u2".into()),
                branch: None,
                type_: None,
                ..Default::default()
            },
        );

        apply_managed_gitignore(root, &cfg).unwrap();

        let root_gi = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(root_gi.contains("projects/mono/"));
        assert!(root_gi.contains("projects/mono/nested/"));
        assert!(root_gi.contains(".odm/cache/"));

        let parent_gi = fs::read_to_string(parent.join(".gitignore")).unwrap();
        assert!(parent_gi.contains("nested/"));
        assert!(!parent_gi.contains(".odm/cache/"));
        assert!(!parent_gi.contains("projects/"));

        // idempotent
        apply_managed_gitignore(root, &cfg).unwrap();
        assert_eq!(
            fs::read_to_string(parent.join(".gitignore")).unwrap(),
            parent_gi
        );
    }

    #[test]
    fn drift_detection() {
        let dir = tempdir().unwrap();
        let cfg = WorkspaceConfig::default();
        assert!(workspace_gitignore_has_drift(dir.path(), &cfg).unwrap());
        apply_managed_gitignore(dir.path(), &cfg).unwrap();
        assert!(!workspace_gitignore_has_drift(dir.path(), &cfg).unwrap());
        // clobber inside markers
        let gi = dir.path().join(".gitignore");
        let text = fs::read_to_string(&gi).unwrap();
        let bad = text.replace(".odm/cache/", ".odm/cache/\nEXTRA/");
        fs::write(&gi, bad).unwrap();
        assert!(workspace_gitignore_has_drift(dir.path(), &cfg).unwrap());
    }

    #[test]
    fn resolve_under_root_blocks_escape() {
        use crate::paths::resolve_under_root;
        let root = Path::new("/ws");
        assert!(resolve_under_root(root, "projects/a").is_ok());
        assert!(resolve_under_root(root, "../outside").is_err());
        assert!(resolve_under_root(root, "/abs").is_err());
    }
}
