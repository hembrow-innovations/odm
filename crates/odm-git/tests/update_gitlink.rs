//! Argv and fixture tests for cacheinfo gitlink stage (does not commit).

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::{Arc, Mutex};

use odm_git::{CommandOutput, CommandRunner, Git, GitError, GitlinkRecord};
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

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

fn abs(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

fn git_user(repo: &Path) {
    assert!(Command::new("git")
        .args([
            "-C",
            repo.to_str().unwrap(),
            "config",
            "user.email",
            "t@est"
        ])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "config", "user.name", "t"])
        .status()
        .unwrap()
        .success());
}

fn commit_file(repo: &Path, name: &str, body: &str) {
    fs::write(repo.join(name), body).unwrap();
    assert!(Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "add", name])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "commit", "-m", name])
        .status()
        .unwrap()
        .success());
}

fn init_repo(path: &Path) {
    fs::create_dir_all(path).unwrap();
    assert!(Command::new("git")
        .args(["init", path.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    git_user(path);
}

fn head_sha(repo: &Path) -> String {
    let out = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout)
        .unwrap()
        .trim()
        .to_ascii_lowercase()
}

#[test]
fn update_gitlink_rejects_relative_repo_without_runner_call() {
    let (runner, calls) = RecordingRunner::ok();
    let g = Git::with_runner(runner);
    let err = g
        .update_gitlink(
            Path::new("rel"),
            Path::new("vendor/nested"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap_err();
    assert!(matches!(err, GitError::NotAbsolute(_)));
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn update_gitlink_argv_cacheinfo_160000() {
    let (runner, calls) = RecordingRunner::ok();
    let g = Git::with_runner(runner);
    g.update_gitlink(
        Path::new("/repo"),
        Path::new("vendor/nested"),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .unwrap();
    let got = args_as_strings(&calls.lock().unwrap()[0]);
    assert_eq!(
        got,
        vec![
            "-C",
            "/repo",
            "update-index",
            "--add",
            "--cacheinfo",
            "160000,aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,vendor/nested"
        ]
    );
    assert!(!got.iter().any(|a| a == "add" || a == "commit"));
}

#[test]
fn update_gitlink_failed_status() {
    let (runner, _) = RecordingRunner::fail("fatal: invalid object");
    let g = Git::with_runner(runner);
    let err = g
        .update_gitlink(
            Path::new("/repo"),
            Path::new("vendor/nested"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap_err();
    match err {
        GitError::Failed {
            operation, stderr, ..
        } => {
            assert_eq!(operation, "update_gitlink");
            assert!(stderr.contains("invalid object"));
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[test]
fn update_gitlink_stages_cacheinfo_and_does_not_commit() {
    let t = TempDir::new().unwrap();
    let repo = abs(&t, "ws");
    init_repo(&repo);
    commit_file(&repo, "README", "hi");
    let before = head_sha(&repo);
    let sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    let g = Git::new();
    g.update_gitlink(&repo, Path::new("vendor/nested"), sha)
        .unwrap();

    assert_eq!(head_sha(&repo), before);
    match g.gitlink_record(&repo, Path::new("vendor/nested")).unwrap() {
        GitlinkRecord::Recorded { sha: got } => assert_eq!(got, sha),
        other => panic!("expected Recorded, got {other:?}"),
    }
}
