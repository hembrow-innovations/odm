use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use odm_git::{Git, GitError, GitlinkRecord};
use tempfile::TempDir;

fn abs(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

fn git_user(repo: &Path) {
    assert!(Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "config", "user.email", "t@est"])
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
    String::from_utf8(out.stdout).unwrap().trim().to_ascii_lowercase()
}

fn stage_gitlink(repo: &Path, rel: &str, sha: &str) {
    let spec = format!("160000,{sha},{rel}");
    assert!(Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "update-index", "--add", "--cacheinfo", &spec])
        .status()
        .unwrap()
        .success());
}

fn stage_gitlink_conflict(repo: &Path, rel: &str, sha1: &str, sha2: &str) {
    let body = format!("160000 {sha1} 1\t{rel}\n160000 {sha2} 2\t{rel}\n");
    let mut child = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "update-index", "--index-info"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(body.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
}

fn sha_has_no_porcelain_prefix(sha: &str) {
    assert_eq!(sha.len(), 40, "sha must be 40 hex chars, got {sha:?}");
    assert!(
        sha.chars().all(|c| c.is_ascii_hexdigit()),
        "sha leaked non-hex porcelain: {sha:?}"
    );
    assert!(!sha.starts_with('-') && !sha.starts_with('+') && !sha.starts_with('U'));
}

#[test]
fn gitlink_record_rejects_relative_repo() {
    let g = Git::new();
    let err = g
        .gitlink_record(Path::new("relative"), Path::new("vendor/nested"))
        .unwrap_err();
    assert!(matches!(err, GitError::NotAbsolute(_)));
}

#[test]
fn gitlink_record_missing_when_path_absent() {
    let t = TempDir::new().unwrap();
    let repo = abs(&t, "ws");
    init_repo(&repo);
    commit_file(&repo, "README", "hi");
    let g = Git::new();
    assert_eq!(
        g.gitlink_record(&repo, Path::new("vendor/nested")).unwrap(),
        GitlinkRecord::Missing
    );
}

#[test]
fn gitlink_record_recorded_has_sha_without_prefix_chars() {
    let t = TempDir::new().unwrap();
    let repo = abs(&t, "ws");
    init_repo(&repo);
    commit_file(&repo, "README", "hi");
    let child = abs(&t, "child");
    init_repo(&child);
    commit_file(&child, "lib.rs", "ok");
    let sha = head_sha(&child);
    stage_gitlink(&repo, "vendor/nested", &sha);

    let g = Git::new();
    match g.gitlink_record(&repo, Path::new("vendor/nested")).unwrap() {
        GitlinkRecord::Recorded { sha: got } => {
            sha_has_no_porcelain_prefix(&got);
            assert_eq!(got, sha);
        }
        other => panic!("expected Recorded, got {other:?}"),
    }
}

#[test]
fn gitlink_record_recorded_when_work_tree_missing() {
    let t = TempDir::new().unwrap();
    let repo = abs(&t, "ws");
    init_repo(&repo);
    commit_file(&repo, "README", "hi");
    let sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    stage_gitlink(&repo, "vendor/nested", sha);
    assert!(!repo.join("vendor/nested").exists());

    let g = Git::new();
    match g.gitlink_record(&repo, Path::new("vendor/nested")).unwrap() {
        GitlinkRecord::Recorded { sha: got } => {
            sha_has_no_porcelain_prefix(&got);
            assert_eq!(got, sha);
        }
        other => panic!("expected Recorded, got {other:?}"),
    }
}

#[test]
fn gitlink_record_conflict_has_no_prefix_chars() {
    let t = TempDir::new().unwrap();
    let repo = abs(&t, "ws");
    init_repo(&repo);
    commit_file(&repo, "README", "hi");
    stage_gitlink_conflict(
        &repo,
        "vendor/nested",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );

    let g = Git::new();
    let rec = g.gitlink_record(&repo, Path::new("vendor/nested")).unwrap();
    assert_eq!(rec, GitlinkRecord::Conflict);
    let debug = format!("{rec:?}");
    assert!(!debug.contains("-aaaaaaaa"));
    assert!(!debug.contains("Uaaaaaaaa"));
    assert!(!debug.contains("+aaaaaaaa"));
}

#[test]
fn gitlink_record_list_gitlinks() {
    let t = TempDir::new().unwrap();
    let repo = abs(&t, "ws");
    init_repo(&repo);
    commit_file(&repo, "README", "hi");
    stage_gitlink(
        &repo,
        "vendor/nested",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    stage_gitlink(
        &repo,
        "vendor/other",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );

    let g = Git::new();
    let mut listed = g.list_gitlinks(&repo).unwrap();
    listed.sort();
    assert_eq!(
        listed,
        vec![PathBuf::from("vendor/nested"), PathBuf::from("vendor/other")]
    );
}

#[test]
fn gitlink_record_unstage_does_not_commit_or_empty_work_tree() {
    let t = TempDir::new().unwrap();
    let repo = abs(&t, "ws");
    init_repo(&repo);
    commit_file(&repo, "README", "hi");
    let nested = repo.join("vendor/nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("kept.txt"), "stay").unwrap();
    stage_gitlink(
        &repo,
        "vendor/nested",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    assert!(Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "commit", "-m", "gitlink"])
        .status()
        .unwrap()
        .success());
    let before = head_sha(&repo);

    let g = Git::new();
    g.unstage_gitlink(&repo, Path::new("vendor/nested")).unwrap();

    assert_eq!(head_sha(&repo), before);
    assert_eq!(fs::read_to_string(nested.join("kept.txt")).unwrap(), "stay");
    assert_eq!(
        g.gitlink_record(&repo, Path::new("vendor/nested")).unwrap(),
        GitlinkRecord::Missing
    );
    assert!(g
        .list_gitlinks(&repo)
        .unwrap()
        .iter()
        .all(|p| p != Path::new("vendor/nested")));
}
