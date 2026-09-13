use super::*;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::process::ExitStatus;
use std::sync::{Arc, Mutex};

use odm_git::{CommandOutput, CommandRunner, Git};
use tempfile::tempdir;

use crate::config::{ProjectEntry, WorkspaceConfig};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

fn exit_ok() -> ExitStatus {
    #[cfg(unix)]
    {
        ExitStatus::from_raw(0)
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new("true").status().unwrap()
    }
}

fn exit_fail(code: i32) -> ExitStatus {
    #[cfg(unix)]
    {
        ExitStatus::from_raw(code << 8)
    }
    #[cfg(not(unix))]
    {
        let _ = code;
        std::process::Command::new("false").status().unwrap()
    }
}

/// Scripted runner: queue of capture results; records argv.
struct ScriptedRunner {
    calls: Arc<Mutex<Vec<Vec<OsString>>>>,
    queue: Mutex<Vec<io::Result<CommandOutput>>>,
}

impl ScriptedRunner {
    fn new(outputs: Vec<CommandOutput>) -> (Self, Arc<Mutex<Vec<Vec<OsString>>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                calls: Arc::clone(&calls),
                queue: Mutex::new(outputs.into_iter().map(Ok).collect()),
            },
            calls,
        )
    }
}

impl CommandRunner for ScriptedRunner {
    fn output(&self, _program: &OsStr, args: &[OsString]) -> io::Result<CommandOutput> {
        self.calls.lock().unwrap().push(args.to_vec());
        let mut q = self.queue.lock().unwrap();
        if q.is_empty() {
            Ok(CommandOutput {
                status: exit_ok(),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        } else {
            q.remove(0)
        }
    }

    fn status(&self, _program: &OsStr, args: &[OsString]) -> io::Result<ExitStatus> {
        self.calls.lock().unwrap().push(args.to_vec());
        Ok(exit_ok())
    }
}

fn out_ok_stdout(stdout: &str) -> CommandOutput {
    CommandOutput {
        status: exit_ok(),
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    }
}

fn out_ok() -> CommandOutput {
    out_ok_stdout("")
}

fn out_fail(stderr: &str) -> CommandOutput {
    CommandOutput {
        status: exit_fail(1),
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

fn is_repo_true() -> CommandOutput {
    out_ok_stdout("true\n")
}

fn is_repo_false() -> CommandOutput {
    // failed status → is_repo returns false
    out_fail("not a repo")
}

fn args_as_strings(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

fn ws_with_project(root: PathBuf, name: &str, rel: &str) -> Workspace {
    let mut projects = BTreeMap::new();
    projects.insert(
        name.into(),
        ProjectEntry {
            path: rel.into(),
            url: None,
            branch: None,
            type_: None,
            ..Default::default()
        },
    );
    Workspace {
        root,
        config: WorkspaceConfig {
            projects,
            ..Default::default()
        },
        actions: BTreeMap::new(),
        generators: BTreeMap::new(),
    }
}

fn ensure_primary(root: &Path, rel: &str) -> PathBuf {
    let p = root.join(rel);
    fs::create_dir_all(&p).unwrap();
    p
}

// --- validate_slot_name ---

#[test]
fn validate_slot_name_accepts_simple() {
    assert_eq!(validate_slot_name("slot1").unwrap(), "slot1");
    assert_eq!(validate_slot_name("  feat-x  ").unwrap(), "feat-x");
}

#[test]
fn validate_slot_name_rejects_empty() {
    assert!(validate_slot_name("").is_err());
    assert!(validate_slot_name("   ").is_err());
}

#[test]
fn validate_slot_name_rejects_dot_and_dotdot() {
    assert!(validate_slot_name(".").is_err());
    assert!(validate_slot_name("..").is_err());
}

#[test]
fn validate_slot_name_rejects_separators_and_nul() {
    assert!(validate_slot_name("a/b").is_err());
    assert!(validate_slot_name("a\\b").is_err());
    assert!(validate_slot_name("a\0b").is_err());
}

#[test]
fn validate_slot_name_rejects_absolute_looking() {
    assert!(validate_slot_name("/abs").is_err());
    assert!(validate_slot_name("C:").is_err());
    assert!(validate_slot_name("C:foo").is_err());
}

// --- resolve / unknown / non-git ---

#[test]
fn unknown_project_is_usage() {
    let dir = tempdir().unwrap();
    let ws = ws_with_project(dir.path().to_path_buf(), "alpha", "projects/alpha");
    let (runner, calls) = ScriptedRunner::new(vec![]);
    let g = Git::with_runner(runner);
    let err = worktree_list(&g, &ws, "missing").unwrap_err();
    assert!(matches!(err, OdmError::Usage(_)));
    assert!(err.to_string().contains("unknown project"));
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn non_git_primary_is_operation() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    ensure_primary(root, "projects/alpha");
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let (runner, _) = ScriptedRunner::new(vec![is_repo_false()]);
    let g = Git::with_runner(runner);
    let err = worktree_list(&g, &ws, "alpha").unwrap_err();
    assert!(matches!(err, OdmError::Operation(_)));
    assert!(err.to_string().contains("not a git repo"));
}

#[test]
fn missing_primary_is_not_found() {
    let dir = tempdir().unwrap();
    let ws = ws_with_project(dir.path().to_path_buf(), "alpha", "projects/alpha");
    let (runner, calls) = ScriptedRunner::new(vec![]);
    let g = Git::with_runner(runner);
    let err = worktree_add(&g, &ws, "alpha", "s1", None).unwrap_err();
    assert!(matches!(err, OdmError::NotFound(_)));
    assert!(calls.lock().unwrap().is_empty());
}

// --- add ---

#[test]
fn add_refuses_existing_path_without_worktree_add() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    ensure_primary(root, "projects/alpha");
    let slot = worktree_slot_path(root, "alpha", "s1");
    fs::create_dir_all(&slot).unwrap();
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let (runner, calls) = ScriptedRunner::new(vec![is_repo_true()]);
    let g = Git::with_runner(runner);
    let err = worktree_add(&g, &ws, "alpha", "s1", None).unwrap_err();
    assert!(matches!(err, OdmError::Operation(_)));
    assert!(err.to_string().contains("already exists"));
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "only is_repo");
    let a = args_as_strings(&calls[0]);
    assert!(a.iter().any(|s| s == "rev-parse"));
}

#[test]
fn add_happy_path_argv() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let slot_path = worktree_slot_path(root, "alpha", "feat");
    let (runner, calls) = ScriptedRunner::new(vec![is_repo_true(), out_ok()]);
    let g = Git::with_runner(runner);
    let out = worktree_add(&g, &ws, "alpha", "feat", Some("topic")).unwrap();
    assert_eq!(out.project, "alpha");
    assert_eq!(out.slot, "feat");
    assert_eq!(out.path, "worktrees/alpha/feat");
    assert!(slot_path.parent().unwrap().is_dir());
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    let add_args = args_as_strings(&calls[1]);
    assert_eq!(
        add_args,
        vec![
            "-C".into(),
            primary.to_string_lossy().into_owned(),
            "worktree".into(),
            "add".into(),
            "-b".into(),
            "topic".into(),
            "--".into(),
            slot_path.to_string_lossy().into_owned(),
        ]
    );
}

#[test]
fn add_rejects_invalid_slot_before_git() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    ensure_primary(root, "projects/alpha");
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let (runner, calls) = ScriptedRunner::new(vec![]);
    let g = Git::with_runner(runner);
    let err = worktree_add(&g, &ws, "alpha", "../x", None).unwrap_err();
    assert!(matches!(err, OdmError::Usage(_)));
    assert!(calls.lock().unwrap().is_empty());
}

// --- list ---

#[test]
fn list_filters_and_sorts_slots() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let s_b = worktree_slot_path(root, "alpha", "b-slot");
    let s_a = worktree_slot_path(root, "alpha", "a-slot");
    let other = root.join("worktrees/other/x");
    let porcelain = format!(
        "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n\
         worktree {}\nHEAD def\nbranch refs/heads/b\n\n\
         worktree {}\nHEAD ghi\nbranch refs/heads/a\n\n\
         worktree {}\nHEAD jkl\nbranch refs/heads/o\n\n",
        primary.display(),
        s_b.display(),
        s_a.display(),
        other.display(),
    );
    let (runner, _) = ScriptedRunner::new(vec![is_repo_true(), out_ok_stdout(&porcelain)]);
    let g = Git::with_runner(runner);
    let out = worktree_list(&g, &ws, "alpha").unwrap();
    assert_eq!(out.project, "alpha");
    assert_eq!(out.slots.len(), 2);
    assert_eq!(out.slots[0].name, "a-slot");
    assert_eq!(out.slots[0].path, "worktrees/alpha/a-slot");
    assert_eq!(out.slots[1].name, "b-slot");
    assert_eq!(out.slots[1].path, "worktrees/alpha/b-slot");
}

#[test]
fn list_empty_is_ok() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let porcelain = format!(
        "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n",
        primary.display()
    );
    let (runner, _) = ScriptedRunner::new(vec![is_repo_true(), out_ok_stdout(&porcelain)]);
    let g = Git::with_runner(runner);
    let out = worktree_list(&g, &ws, "alpha").unwrap();
    assert!(out.slots.is_empty());
}

#[test]
fn list_sets_dirty_from_is_clean() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    let clean = worktree_slot_path(root, "alpha", "clean");
    let dirty = worktree_slot_path(root, "alpha", "dirty");
    let unknown = worktree_slot_path(root, "alpha", "unknown");
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let porcelain = format!(
        "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n\
         worktree {}\nHEAD def\nbranch refs/heads/c\n\n\
         worktree {}\nHEAD ghi\nbranch refs/heads/d\n\n\
         worktree {}\nHEAD jkl\nbranch refs/heads/u\n\n",
        primary.display(),
        clean.display(),
        dirty.display(),
        unknown.display(),
    );
    // sorted: clean, dirty, unknown
    let (runner, _) = ScriptedRunner::new(vec![
        is_repo_true(),
        out_ok_stdout(&porcelain),
        out_ok_stdout(""),             // clean → dirty false
        out_ok_stdout(" M x\n"),       // dirty → dirty true
        out_fail("status failed"),     // unknown → None
    ]);
    let g = Git::with_runner(runner);
    let out = worktree_list(&g, &ws, "alpha").unwrap();
    assert_eq!(out.slots.len(), 3);
    assert_eq!(out.slots[0].name, "clean");
    assert_eq!(out.slots[0].dirty, Some(false));
    assert_eq!(out.slots[1].name, "dirty");
    assert_eq!(out.slots[1].dirty, Some(true));
    assert_eq!(out.slots[2].name, "unknown");
    assert_eq!(out.slots[2].dirty, None);
    let v = serde_json::to_value(&out.slots[1]).unwrap();
    assert_eq!(v["dirty"], true);
    let v = serde_json::to_value(&out.slots[2]).unwrap();
    assert!(v["dirty"].is_null(), "{v}");
}

// --- rm ---

#[test]
fn rm_missing_slot_is_not_found() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    ensure_primary(root, "projects/alpha");
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let (runner, calls) = ScriptedRunner::new(vec![is_repo_true()]);
    let g = Git::with_runner(runner);
    let err = worktree_rm(&g, &ws, "alpha", "gone", false).unwrap_err();
    assert!(matches!(err, OdmError::NotFound(_)));
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "only is_repo; no list/remove");
}

#[test]
fn rm_force_passthrough_argv() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    let slot_path = worktree_slot_path(root, "alpha", "s1");
    fs::create_dir_all(&slot_path).unwrap();
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let porcelain = format!(
        "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n\
         worktree {}\nHEAD def\nbranch refs/heads/s\n\n",
        primary.display(),
        slot_path.display(),
    );
    let (runner, calls) = ScriptedRunner::new(vec![
        is_repo_true(),
        out_ok_stdout(&porcelain),
        out_ok(),
    ]);
    let g = Git::with_runner(runner);
    let out = worktree_rm(&g, &ws, "alpha", "s1", true).unwrap();
    assert_eq!(out.path, "worktrees/alpha/s1");
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    let rm_args = args_as_strings(&calls[2]);
    assert_eq!(
        rm_args,
        vec![
            "-C".into(),
            primary.to_string_lossy().into_owned(),
            "worktree".into(),
            "remove".into(),
            "--force".into(),
            "--".into(),
            slot_path.to_string_lossy().into_owned(),
        ]
    );
}

// --- prune ---

fn porcelain_primary_only(primary: &Path) -> String {
    format!(
        "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n",
        primary.display()
    )
}

fn porcelain_with_slot(primary: &Path, slot_path: &Path) -> String {
    format!(
        "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n\
         worktree {}\nHEAD def\nbranch refs/heads/s\n\n",
        primary.display(),
        slot_path.display(),
    )
}

#[test]
fn prune_removes_empty_orphan_without_force() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    let orphan = worktree_slot_path(root, "alpha", "stale");
    fs::create_dir_all(&orphan).unwrap();
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let (runner, _) =
        ScriptedRunner::new(vec![is_repo_true(), out_ok_stdout(&porcelain_primary_only(&primary))]);
    let g = Git::with_runner(runner);
    let out = worktree_prune(&g, &ws, "alpha", false).unwrap();
    assert_eq!(out.project, "alpha");
    assert_eq!(out.pruned.len(), 1);
    assert_eq!(out.pruned[0].name, "stale");
    assert_eq!(out.pruned[0].path, "worktrees/alpha/stale");
    assert!(out.skipped_nonempty.is_empty());
    assert!(!orphan.exists());
}

#[test]
fn prune_skips_nonempty_without_force_removes_empties() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    let empty = worktree_slot_path(root, "alpha", "empty");
    let full = worktree_slot_path(root, "alpha", "full");
    fs::create_dir_all(&empty).unwrap();
    fs::create_dir_all(&full).unwrap();
    fs::write(full.join("leftover.txt"), "x").unwrap();
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let (runner, _) =
        ScriptedRunner::new(vec![is_repo_true(), out_ok_stdout(&porcelain_primary_only(&primary))]);
    let g = Git::with_runner(runner);
    let out = worktree_prune(&g, &ws, "alpha", false).unwrap();
    assert_eq!(out.pruned.len(), 1);
    assert_eq!(out.pruned[0].name, "empty");
    assert_eq!(out.skipped_nonempty.len(), 1);
    assert_eq!(out.skipped_nonempty[0].name, "full");
    assert!(!empty.exists());
    assert!(full.is_dir());
    assert!(full.join("leftover.txt").is_file());
}

#[test]
fn prune_force_removes_nonempty_orphan() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    let full = worktree_slot_path(root, "alpha", "full");
    fs::create_dir_all(&full).unwrap();
    fs::write(full.join("leftover.txt"), "x").unwrap();
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let (runner, _) =
        ScriptedRunner::new(vec![is_repo_true(), out_ok_stdout(&porcelain_primary_only(&primary))]);
    let g = Git::with_runner(runner);
    let out = worktree_prune(&g, &ws, "alpha", true).unwrap();
    assert_eq!(out.pruned.len(), 1);
    assert_eq!(out.pruned[0].name, "full");
    assert!(out.skipped_nonempty.is_empty());
    assert!(!full.exists());
}

#[test]
fn prune_never_deletes_registered_slot_even_with_force() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    let registered = worktree_slot_path(root, "alpha", "live");
    let orphan = worktree_slot_path(root, "alpha", "stale");
    fs::create_dir_all(&registered).unwrap();
    fs::write(registered.join("keep.txt"), "reg").unwrap();
    fs::create_dir_all(&orphan).unwrap();
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let porcelain = porcelain_with_slot(&primary, &registered);
    let (runner, _) = ScriptedRunner::new(vec![is_repo_true(), out_ok_stdout(&porcelain)]);
    let g = Git::with_runner(runner);
    let out = worktree_prune(&g, &ws, "alpha", true).unwrap();
    assert_eq!(out.pruned.len(), 1);
    assert_eq!(out.pruned[0].name, "stale");
    assert!(!orphan.exists());
    assert!(registered.is_dir());
    assert!(registered.join("keep.txt").is_file());
}

#[test]
fn prune_no_orphans_is_empty_ok() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let (runner, _) =
        ScriptedRunner::new(vec![is_repo_true(), out_ok_stdout(&porcelain_primary_only(&primary))]);
    let g = Git::with_runner(runner);
    let out = worktree_prune(&g, &ws, "alpha", false).unwrap();
    assert!(out.pruned.is_empty());
    assert!(out.skipped_nonempty.is_empty());
}

#[test]
fn prune_unknown_project_is_usage() {
    let dir = tempdir().unwrap();
    let ws = ws_with_project(dir.path().to_path_buf(), "alpha", "projects/alpha");
    let (runner, calls) = ScriptedRunner::new(vec![]);
    let g = Git::with_runner(runner);
    let err = worktree_prune(&g, &ws, "missing", false).unwrap_err();
    assert!(matches!(err, OdmError::Usage(_)));
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn prune_non_git_primary_is_operation() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    ensure_primary(root, "projects/alpha");
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let (runner, _) = ScriptedRunner::new(vec![is_repo_false()]);
    let g = Git::with_runner(runner);
    let err = worktree_prune(&g, &ws, "alpha", false).unwrap_err();
    assert!(matches!(err, OdmError::Operation(_)));
}

#[test]
fn prune_skips_invalid_names_and_files() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    let project_wt = root.join("worktrees/alpha");
    fs::create_dir_all(&project_wt).unwrap();
    fs::write(project_wt.join("not-a-dir"), "x").unwrap();
    // ".." is invalid slot name — create via join would escape; skip creating ".."
    let real = worktree_slot_path(root, "alpha", "real");
    fs::create_dir_all(&real).unwrap();
    let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
    let (runner, _) =
        ScriptedRunner::new(vec![is_repo_true(), out_ok_stdout(&porcelain_primary_only(&primary))]);
    let g = Git::with_runner(runner);
    let out = worktree_prune(&g, &ws, "alpha", false).unwrap();
    assert_eq!(out.pruned.len(), 1);
    assert_eq!(out.pruned[0].name, "real");
    assert!(project_wt.join("not-a-dir").is_file());
}

fn ws_with_projects(root: PathBuf, projects: &[(&str, &str)]) -> Workspace {
    let mut map = BTreeMap::new();
    for (name, rel) in projects {
        map.insert(
            (*name).into(),
            ProjectEntry {
                path: (*rel).into(),
                url: None,
                branch: None,
                type_: None,
                ..Default::default()
            },
        );
    }
    Workspace {
        root,
        config: WorkspaceConfig {
            projects: map,
            ..Default::default()
        },
        actions: BTreeMap::new(),
        generators: BTreeMap::new(),
    }
}

#[test]
fn prune_all_removes_empty_orphans_across_projects() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let alpha = ensure_primary(root, "projects/alpha");
    let beta = ensure_primary(root, "projects/beta");
    let a_orphan = worktree_slot_path(root, "alpha", "stale");
    let b_orphan = worktree_slot_path(root, "beta", "stale");
    fs::create_dir_all(&a_orphan).unwrap();
    fs::create_dir_all(&b_orphan).unwrap();
    let ws = ws_with_projects(
        root.to_path_buf(),
        &[("alpha", "projects/alpha"), ("beta", "projects/beta")],
    );
    // BTreeMap order: alpha then beta — each needs is_repo + list
    let (runner, _) = ScriptedRunner::new(vec![
        is_repo_true(),
        out_ok_stdout(&porcelain_primary_only(&alpha)),
        is_repo_true(),
        out_ok_stdout(&porcelain_primary_only(&beta)),
    ]);
    let g = Git::with_runner(runner);
    let out = worktree_prune_all(&g, &ws, false).unwrap();
    assert_eq!(out.pruned.len(), 2);
    assert_eq!(out.pruned[0].project, "alpha");
    assert_eq!(out.pruned[0].name, "stale");
    assert_eq!(out.pruned[0].path, "worktrees/alpha/stale");
    assert_eq!(out.pruned[1].project, "beta");
    assert_eq!(out.pruned[1].name, "stale");
    assert!(out.skipped_nonempty.is_empty());
    assert!(!a_orphan.exists());
    assert!(!b_orphan.exists());
}

#[test]
fn prune_all_skips_non_git_and_missing_primary() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let alpha = ensure_primary(root, "projects/alpha");
    let a_orphan = worktree_slot_path(root, "alpha", "stale");
    fs::create_dir_all(&a_orphan).unwrap();
    // beta path missing; gamma exists but non-git
    ensure_primary(root, "projects/gamma");
    let ws = ws_with_projects(
        root.to_path_buf(),
        &[
            ("alpha", "projects/alpha"),
            ("beta", "projects/beta"),
            ("gamma", "projects/gamma"),
        ],
    );
    // alpha: is_repo + list; beta: missing path → NotFound before git; gamma: is_repo false
    let (runner, _) = ScriptedRunner::new(vec![
        is_repo_true(),
        out_ok_stdout(&porcelain_primary_only(&alpha)),
        is_repo_false(),
    ]);
    let g = Git::with_runner(runner);
    let out = worktree_prune_all(&g, &ws, false).unwrap();
    assert_eq!(out.pruned.len(), 1);
    assert_eq!(out.pruned[0].project, "alpha");
    assert!(out.skipped_nonempty.is_empty());
    assert!(!a_orphan.exists());
}

#[test]
fn prune_all_partial_nonempty_and_force_registered_safe() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let alpha = ensure_primary(root, "projects/alpha");
    let beta = ensure_primary(root, "projects/beta");
    let a_empty = worktree_slot_path(root, "alpha", "empty");
    let b_full = worktree_slot_path(root, "beta", "full");
    let b_live = worktree_slot_path(root, "beta", "live");
    fs::create_dir_all(&a_empty).unwrap();
    fs::create_dir_all(&b_full).unwrap();
    fs::write(b_full.join("leftover.txt"), "x").unwrap();
    fs::create_dir_all(&b_live).unwrap();
    fs::write(b_live.join("keep.txt"), "reg").unwrap();
    let ws = ws_with_projects(
        root.to_path_buf(),
        &[("alpha", "projects/alpha"), ("beta", "projects/beta")],
    );
    let beta_porcelain = porcelain_with_slot(&beta, &b_live);
    // without force: alpha empty pruned; beta full skipped; live untouched
    let (runner, _) = ScriptedRunner::new(vec![
        is_repo_true(),
        out_ok_stdout(&porcelain_primary_only(&alpha)),
        is_repo_true(),
        out_ok_stdout(&beta_porcelain),
    ]);
    let g = Git::with_runner(runner);
    let out = worktree_prune_all(&g, &ws, false).unwrap();
    assert_eq!(out.pruned.len(), 1);
    assert_eq!(out.pruned[0].project, "alpha");
    assert_eq!(out.skipped_nonempty.len(), 1);
    assert_eq!(out.skipped_nonempty[0].project, "beta");
    assert_eq!(out.skipped_nonempty[0].name, "full");
    assert!(!a_empty.exists());
    assert!(b_full.is_dir());
    assert!(b_live.join("keep.txt").is_file());

    // force: remove full; live still safe
    let (runner, _) = ScriptedRunner::new(vec![
        is_repo_true(),
        out_ok_stdout(&porcelain_primary_only(&alpha)),
        is_repo_true(),
        out_ok_stdout(&beta_porcelain),
    ]);
    let g = Git::with_runner(runner);
    let out = worktree_prune_all(&g, &ws, true).unwrap();
    assert_eq!(out.pruned.len(), 1);
    assert_eq!(out.pruned[0].name, "full");
    assert!(out.skipped_nonempty.is_empty());
    assert!(!b_full.exists());
    assert!(b_live.join("keep.txt").is_file());
}
