//! Argv tests for submodule add and path-limited init (no real git).

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::ExitStatus;
use std::sync::{Arc, Mutex};

use odm_git::{CommandOutput, CommandRunner, Git, GitError};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

#[cfg(not(unix))]
use std::process::Command;

fn exit_ok() -> ExitStatus {
    #[cfg(unix)]
    {
        ExitStatus::from_raw(0)
    }
    #[cfg(not(unix))]
    {
        Command::new("true").status().unwrap()
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
        Command::new("false").status().unwrap()
    }
}

struct RecordingRunner {
    calls: Arc<Mutex<Vec<Vec<OsString>>>>,
    next: Mutex<Option<io::Result<CommandOutput>>>,
}

impl RecordingRunner {
    fn new(out: CommandOutput) -> (Self, Arc<Mutex<Vec<Vec<OsString>>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                calls: Arc::clone(&calls),
                next: Mutex::new(Some(Ok(out))),
            },
            calls,
        )
    }

    fn ok() -> (Self, Arc<Mutex<Vec<Vec<OsString>>>>) {
        Self::new(CommandOutput {
            status: exit_ok(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }

    fn fail(operation_stderr: &str) -> (Self, Arc<Mutex<Vec<Vec<OsString>>>>) {
        Self::new(CommandOutput {
            status: exit_fail(1),
            stdout: Vec::new(),
            stderr: operation_stderr.as_bytes().to_vec(),
        })
    }
}

impl CommandRunner for RecordingRunner {
    fn output(&self, _program: &OsStr, args: &[OsString]) -> io::Result<CommandOutput> {
        self.calls.lock().unwrap().push(args.to_vec());
        self.next.lock().unwrap().take().unwrap_or_else(|| {
            Ok(CommandOutput {
                status: exit_ok(),
                stdout: Vec::new(),
                stderr: Vec::new(),
            })
        })
    }

    fn status(&self, _program: &OsStr, args: &[OsString]) -> io::Result<ExitStatus> {
        self.calls.lock().unwrap().push(args.to_vec());
        Ok(exit_ok())
    }
}

fn args_as_strings(args: &[OsString]) -> Vec<String> {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

fn assert_no_recurse_or_remote(args: &[String]) {
    for a in args {
        let lower = a.to_ascii_lowercase();
        assert!(
            !lower.contains("recurse"),
            "recurse leaked in argv: {args:?}"
        );
        assert_ne!(lower, "--remote", "remote leaked in argv: {args:?}");
        assert_ne!(lower, "-remote", "remote leaked in argv: {args:?}");
    }
}

#[test]
fn submodule_add_rejects_relative_repo_without_runner_call() {
    let (runner, calls) = RecordingRunner::ok();
    let g = Git::with_runner(runner);
    let err = g
        .submodule_add(
            Path::new("rel"),
            "https://example.com/n.git",
            Path::new("vendor/nested"),
            None,
        )
        .unwrap_err();
    assert!(matches!(err, GitError::NotAbsolute(_)));
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn submodule_add_argv_without_branch() {
    let (runner, calls) = RecordingRunner::ok();
    let g = Git::with_runner(runner);
    g.submodule_add(
        Path::new("/repo"),
        "https://example.com/n.git",
        Path::new("vendor/nested"),
        None,
    )
    .unwrap();
    let got = args_as_strings(&calls.lock().unwrap()[0]);
    assert_eq!(
        got,
        vec![
            "-C",
            "/repo",
            "submodule",
            "add",
            "--",
            "https://example.com/n.git",
            "vendor/nested"
        ]
    );
    assert_no_recurse_or_remote(&got);
}

#[test]
fn submodule_add_argv_with_branch() {
    let (runner, calls) = RecordingRunner::ok();
    let g = Git::with_runner(runner);
    g.submodule_add(
        Path::new("/repo"),
        "https://example.com/n.git",
        Path::new("vendor/nested"),
        Some("main"),
    )
    .unwrap();
    let got = args_as_strings(&calls.lock().unwrap()[0]);
    assert_eq!(
        got,
        vec![
            "-C",
            "/repo",
            "submodule",
            "add",
            "-b",
            "main",
            "--",
            "https://example.com/n.git",
            "vendor/nested"
        ]
    );
    assert_no_recurse_or_remote(&got);
}

#[test]
fn submodule_add_does_not_commit() {
    let (runner, calls) = RecordingRunner::ok();
    let g = Git::with_runner(runner);
    g.submodule_add(
        Path::new("/repo"),
        "https://example.com/n.git",
        Path::new("vendor/nested"),
        None,
    )
    .unwrap();
    let got = args_as_strings(&calls.lock().unwrap()[0]);
    assert!(!got.iter().any(|a| a == "commit"));
}

#[test]
fn submodule_add_failed_status() {
    let (runner, _) = RecordingRunner::fail("fatal: already exists");
    let g = Git::with_runner(runner);
    let err = g
        .submodule_add(
            Path::new("/repo"),
            "https://example.com/n.git",
            Path::new("vendor/nested"),
            None,
        )
        .unwrap_err();
    match err {
        GitError::Failed {
            operation, stderr, ..
        } => {
            assert_eq!(operation, "submodule_add");
            assert!(stderr.contains("already exists"));
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn submodule_add_update_init_rejects_relative_repo_without_runner_call() {
    let (runner, calls) = RecordingRunner::ok();
    let g = Git::with_runner(runner);
    let err = g
        .submodule_update_init(Path::new("rel"), Path::new("vendor/nested"))
        .unwrap_err();
    assert!(matches!(err, GitError::NotAbsolute(_)));
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn submodule_add_update_init_argv_path_limited() {
    let (runner, calls) = RecordingRunner::ok();
    let g = Git::with_runner(runner);
    g.submodule_update_init(Path::new("/repo"), Path::new("vendor/nested"))
        .unwrap();
    let got = args_as_strings(&calls.lock().unwrap()[0]);
    assert_eq!(
        got,
        vec![
            "-C",
            "/repo",
            "submodule",
            "update",
            "--init",
            "--",
            "vendor/nested"
        ]
    );
    assert_no_recurse_or_remote(&got);
    assert!(!got.iter().any(|a| a == "commit"));
}

#[test]
fn submodule_add_update_init_failed_status() {
    let (runner, _) = RecordingRunner::fail("fatal: pathspec");
    let g = Git::with_runner(runner);
    let err = g
        .submodule_update_init(Path::new("/repo"), Path::new("vendor/nested"))
        .unwrap_err();
    match err {
        GitError::Failed { operation, .. } => assert_eq!(operation, "submodule_update_init"),
        other => panic!("expected Failed, got {other:?}"),
    }
}
