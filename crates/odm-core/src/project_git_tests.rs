use super::*;
use crate::config::{ProjectEntry, Workspace, WorkspaceConfig};
use crate::error::OdmError;
use crate::paths::{pin_path, worktree_slot_path};
use crate::pin::{PinEntry, PinFile};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

use odm_git::{CommandOutput, CommandRunner, Git};

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

fn is_repo_true() -> CommandOutput {
    out_ok_stdout("true\n")
}

fn args_as_strings(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

fn ws_with_project(root: PathBuf, name: &str, rel: &str, url: Option<&str>) -> Workspace {
    let mut projects = BTreeMap::new();
    projects.insert(
        name.into(),
        ProjectEntry {
            path: rel.into(),
            url: url.map(|s| s.into()),
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

fn git_args(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn project_git_wt_runs_in_slot_path() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let ws = ws_with_project(root.clone(), "alpha", "projects/alpha", None);
    let slot = worktree_slot_path(&root, "alpha", "feat");
    fs::create_dir_all(&slot).unwrap();

    let (runner, calls) = ScriptedRunner::new(vec![is_repo_true()]);
    let git = Git::with_runner(runner);
    let status = project_git(
        &git,
        &ws,
        "alpha",
        &git_args(&["status"]),
        Some("feat"),
    )
    .unwrap();
    assert!(status.success());

    let recorded = calls.lock().unwrap();
    // is_repo (output) then run (status)
    assert_eq!(recorded.len(), 2);
    let run_args = args_as_strings(&recorded[1]);
    assert_eq!(run_args[0], "-C");
    assert_eq!(PathBuf::from(&run_args[1]), slot);
    assert_eq!(run_args[2], "status");
}

#[test]
fn project_git_wt_missing_slot_is_not_found() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let ws = ws_with_project(root, "alpha", "projects/alpha", None);
    let (runner, _) = ScriptedRunner::new(vec![]);
    let git = Git::with_runner(runner);
    let err = project_git(
        &git,
        &ws,
        "alpha",
        &git_args(&["status"]),
        Some("missing"),
    )
    .unwrap_err();
    assert!(matches!(err, OdmError::NotFound(_)));
    assert!(!err.to_string().contains("not implemented"));
    assert!(err.to_string().contains("worktree slot not found"));
}

#[test]
fn project_git_wt_invalid_slot_name_is_usage() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let ws = ws_with_project(root, "alpha", "projects/alpha", None);
    let (runner, _) = ScriptedRunner::new(vec![]);
    let git = Git::with_runner(runner);
    let err = project_git(
        &git,
        &ws,
        "alpha",
        &git_args(&["status"]),
        Some("a/b"),
    )
    .unwrap_err();
    assert!(matches!(err, OdmError::Usage(_)));
}

#[test]
fn project_git_wt_unknown_project_is_usage() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let ws = ws_with_project(root, "alpha", "projects/alpha", None);
    let (runner, _) = ScriptedRunner::new(vec![]);
    let git = Git::with_runner(runner);
    let err = project_git(
        &git,
        &ws,
        "nope",
        &git_args(&["status"]),
        Some("feat"),
    )
    .unwrap_err();
    assert!(matches!(err, OdmError::Usage(_)));
    assert!(err.to_string().contains("unknown project"));
}

#[test]
fn project_git_none_wt_uses_primary_path() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let primary = root.join("projects/alpha");
    fs::create_dir_all(&primary).unwrap();
    let ws = ws_with_project(root, "alpha", "projects/alpha", None);

    let (runner, calls) = ScriptedRunner::new(vec![
        is_repo_true(),
        // head_sha before (ok → ignored if fail; provide valid-looking)
        out_ok_stdout("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n"),
        // head_sha after (same → no pin maintain)
        out_ok_stdout("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n"),
    ]);
    let git = Git::with_runner(runner);
    let status = project_git(&git, &ws, "alpha", &git_args(&["status"]), None).unwrap();
    assert!(status.success());

    let recorded = calls.lock().unwrap();
    // is_repo, head_sha before, run, head_sha after
    let run = recorded
        .iter()
        .map(|a| args_as_strings(a))
        .find(|a| a.get(2).map(|s| s.as_str()) == Some("status"))
        .expect("run status call");
    assert_eq!(run[0], "-C");
    assert_eq!(PathBuf::from(&run[1]), primary);
}

#[test]
fn project_git_wt_does_not_update_pin() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let primary = root.join("projects/alpha");
    fs::create_dir_all(&primary).unwrap();
    let slot = worktree_slot_path(&root, "alpha", "feat");
    fs::create_dir_all(&slot).unwrap();

    let old_sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let mut pin = PinFile::new_v1();
    pin.pins.insert(
        "alpha".into(),
        PinEntry {
            rev: old_sha.into(),
            url: "https://example.com/alpha.git".into(),
            branch: Some("main".into()),
        },
    );
    // Ensure pin parent exists.
    fs::create_dir_all(root.join(".odm")).unwrap();
    crate::pin::save_pin(&root, &pin).unwrap();
    let before_bytes = fs::read(pin_path(&root)).unwrap();

    let ws = ws_with_project(
        root.clone(),
        "alpha",
        "projects/alpha",
        Some("https://example.com/alpha.git"),
    );
    let (runner, calls) = ScriptedRunner::new(vec![is_repo_true()]);
    let git = Git::with_runner(runner);
    project_git(
        &git,
        &ws,
        "alpha",
        &git_args(&["checkout", "other"]),
        Some("feat"),
    )
    .unwrap();

    let after_bytes = fs::read(pin_path(&root)).unwrap();
    assert_eq!(
        before_bytes, after_bytes,
        "pin file must not change with --wt"
    );

    // Only is_repo + run — no head_sha / pin maintain git calls.
    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 2);
    let run_args = args_as_strings(&recorded[1]);
    assert_eq!(PathBuf::from(&run_args[1]), slot);
}
