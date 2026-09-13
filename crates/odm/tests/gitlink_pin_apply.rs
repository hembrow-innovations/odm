//! CLI: `odm pin apply` restores gitlink SHA. `odm.git:gitlink-pin-index` `odm.cli:pin-apply` `odm.cli:exit-codes`

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use assert_cmd::cargo::cargo_bin;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::tempdir;

fn odm() -> assert_cmd::Command {
    assert_cmd::Command::new(cargo_bin("odm"))
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

fn bare_with_main(root: &Path, name: &str) -> PathBuf {
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

fn odm_file_protocol() -> assert_cmd::Command {
    let mut c = odm();
    c.env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "protocol.file.allow")
        .env("GIT_CONFIG_VALUE_0", "always");
    c
}

fn commit_workspace(root: &Path) {
    git_user(root);
    fs::write(root.join("README"), "ws").unwrap();
    assert!(Command::new("git")
        .args(["-C", root.to_str().unwrap(), "add", "README"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["-C", root.to_str().unwrap(), "commit", "-m", "init"])
        .status()
        .unwrap()
        .success());
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

fn stage_row(repo: &Path, rel: &str) -> String {
    let out = Command::new("git")
        .args([
            "-C",
            repo.to_str().unwrap(),
            "ls-files",
            "--stage",
            "--",
            rel,
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

fn gitlink_sha(repo: &Path, rel: &str) -> String {
    stage_row(repo, rel)
        .split_whitespace()
        .nth(1)
        .expect("gitlink sha")
        .to_ascii_lowercase()
}

fn is_detached(repo: &Path) -> bool {
    !Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "symbolic-ref", "-q", "HEAD"])
        .status()
        .unwrap()
        .success()
}

fn ws_git_committed() -> (tempfile::TempDir, PathBuf) {
    let dir = tempdir().unwrap();
    let root = dir.path().join("ws");
    odm()
        .args(["init", root.to_str().unwrap()])
        .assert()
        .success();
    commit_workspace(&root);
    (dir, root)
}

fn add_gitlink_nested(dir: &Path, root: &Path) {
    let bare = bare_with_main(dir, "nested");
    odm_file_protocol()
        .args([
            "--root",
            root.to_str().unwrap(),
            "project",
            "add",
            "nested",
            "--path",
            "vendor/nested",
            "--url",
            bare.to_str().unwrap(),
            "--branch",
            "main",
            "--gitlink",
        ])
        .assert()
        .success();
}

fn commit_child(nested: &Path, name: &str, body: &str) {
    git_user(nested);
    fs::write(nested.join(name), body).unwrap();
    assert!(Command::new("git")
        .args(["-C", nested.to_str().unwrap(), "add", name])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["-C", nested.to_str().unwrap(), "commit", "-m", name])
        .status()
        .unwrap()
        .success());
}

/// `odm.git:gitlink-pin-index` `odm.cli:pin-apply`
#[test]
fn gitlink_pin_apply_restores_sha() {
    let (dir, root) = ws_git_committed();
    add_gitlink_nested(dir.path(), &root);
    let root_s = root.to_str().unwrap();
    let nested = root.join("vendor/nested");
    let parent_before = head_sha(&root);
    let recorded = gitlink_sha(&root, "vendor/nested");
    commit_child(&nested, "MORE", "x");
    assert_ne!(head_sha(&nested), recorded);

    let out = odm()
        .args(["--root", root_s, "--json", "pin", "apply", "nested"])
        .assert()
        .success()
        .get_output()
        .clone();
    let v: Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    let results = v["results"].as_array().expect("results");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "nested");
    assert_eq!(results[0]["status"], "applied");
    assert_eq!(results[0]["rev"].as_str().unwrap(), recorded);
    assert_eq!(results[0]["detached"], true);

    assert_eq!(head_sha(&nested), recorded);
    assert!(is_detached(&nested));
    assert_eq!(
        gitlink_sha(&root, "vendor/nested"),
        recorded,
        "must not stage"
    );
    assert_eq!(
        head_sha(&root),
        parent_before,
        "must not commit workspace root"
    );
    let lock = root.join(".odm/odm.lock.yaml");
    assert!(
        !lock.exists() || !fs::read_to_string(&lock).unwrap().contains("nested"),
        "pin file must not list gitlink name"
    );
}

/// `odm.cli:pin-apply` `odm.cli:exit-codes`
#[test]
fn gitlink_pin_apply_refuses_dirty() {
    let (dir, root) = ws_git_committed();
    add_gitlink_nested(dir.path(), &root);
    let root_s = root.to_str().unwrap();
    let nested = root.join("vendor/nested");
    let recorded = gitlink_sha(&root, "vendor/nested");
    commit_child(&nested, "MORE", "x");
    let drifted = head_sha(&nested);
    fs::write(nested.join("dirty"), "x").unwrap();

    odm()
        .args(["--root", root_s, "pin", "apply", "nested"])
        .assert()
        .failure()
        .code(3)
        .stderr(predicate::str::contains("dirty"));

    assert_eq!(head_sha(&nested), drifted);
    assert_eq!(gitlink_sha(&root, "vendor/nested"), recorded);

    odm()
        .args(["--root", root_s, "pin", "apply", "nested", "--force"])
        .assert()
        .success();

    assert_eq!(head_sha(&nested), recorded);
    assert!(is_detached(&nested));
    assert_eq!(
        gitlink_sha(&root, "vendor/nested"),
        recorded,
        "must not stage"
    );
}

/// `odm.cli:pin-apply` `odm.cli:exit-codes`
#[test]
fn gitlink_pin_apply_missing_is_not_found() {
    let (dir, root) = ws_git_committed();
    add_gitlink_nested(dir.path(), &root);
    let nested = root.join("vendor/nested");
    let child_before = head_sha(&nested);
    assert!(Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap(),
            "rm",
            "--cached",
            "vendor/nested"
        ])
        .status()
        .unwrap()
        .success());
    let root_s = root.to_str().unwrap();

    let out = odm()
        .args(["--root", root_s, "--json", "pin", "apply", "nested"])
        .assert()
        .failure()
        .code(4)
        .get_output()
        .clone();
    let v: Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "not_found");
    assert_eq!(head_sha(&nested), child_before);
}

/// `odm.git:gitlink-pin-index` `odm.git:pin-sha` `odm.cli:pin-apply`
#[test]
fn gitlink_pin_apply_mixed_uses_pin_source() {
    let (dir, root) = ws_git_committed();
    add_gitlink_nested(dir.path(), &root);
    let clone_bare = bare_with_main(dir.path(), "alpha");
    odm_file_protocol()
        .args([
            "--root",
            root.to_str().unwrap(),
            "project",
            "add",
            "alpha",
            "--path",
            "projects/alpha",
            "--url",
            clone_bare.to_str().unwrap(),
            "--branch",
            "main",
        ])
        .assert()
        .success();
    let root_s = root.to_str().unwrap();
    let nested = root.join("vendor/nested");
    let alpha = root.join("projects/alpha");
    let recorded = gitlink_sha(&root, "vendor/nested");
    let pin = json_stdout(odm().args(["--root", root_s, "--json", "pin", "status", "alpha"]));
    let lock_rev = pin["entries"][0]["pin_rev"]
        .as_str()
        .expect("alpha pin_rev")
        .to_ascii_lowercase();
    commit_child(&nested, "MORE", "x");
    git_user(&alpha);
    fs::write(alpha.join("DRIFT"), "y").unwrap();
    assert!(Command::new("git")
        .args(["-C", alpha.to_str().unwrap(), "add", "DRIFT"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["-C", alpha.to_str().unwrap(), "commit", "-m", "drift"])
        .status()
        .unwrap()
        .success());
    assert_ne!(head_sha(&nested), recorded);
    assert_ne!(head_sha(&alpha), lock_rev);

    let out = odm()
        .args([
            "--root", root_s, "--json", "pin", "apply", "nested", "alpha",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let v: Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    let results = v["results"].as_array().expect("results");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["name"], "nested");
    assert_eq!(results[0]["rev"].as_str().unwrap(), recorded);
    assert_eq!(results[0]["detached"], true);
    assert_eq!(results[1]["name"], "alpha");
    assert_eq!(results[1]["rev"].as_str().unwrap(), lock_rev);
    assert_eq!(results[1]["detached"], true);

    assert_eq!(head_sha(&nested), recorded);
    assert!(is_detached(&nested));
    assert_eq!(head_sha(&alpha), lock_rev);
    assert!(is_detached(&alpha));
    assert_eq!(
        gitlink_sha(&root, "vendor/nested"),
        recorded,
        "must not stage"
    );
    let lock = fs::read_to_string(root.join(".odm/odm.lock.yaml")).unwrap();
    assert!(lock.contains("alpha"));
    assert!(
        !lock.contains("nested"),
        "pin file must not list gitlink name"
    );
}

fn json_stdout(cmd: &mut assert_cmd::Command) -> Value {
    let out = cmd.assert().success().get_output().stdout.clone();
    serde_json::from_slice(&out).expect("stdout JSON")
}

/// `odm.git:gitlink-pin-index` leftover lock keys must not be apply authority.
#[test]
fn gitlink_pin_apply_empty_ignores_lock_key() {
    let (dir, root) = ws_git_committed();
    add_gitlink_nested(dir.path(), &root);
    let root_s = root.to_str().unwrap();
    let nested = root.join("vendor/nested");
    let recorded = gitlink_sha(&root, "vendor/nested");
    let fake = "b".repeat(40);
    fs::write(
        root.join(".odm/odm.lock.yaml"),
        format!(
            "version: 1\npins:\n  nested:\n    rev: {fake}\n    url: https://example.com/nested.git\n"
        ),
    )
    .unwrap();
    commit_child(&nested, "MORE", "x");
    assert_ne!(head_sha(&nested), recorded);

    odm()
        .args(["--root", root_s, "pin", "apply"])
        .assert()
        .success();

    assert_eq!(head_sha(&nested), recorded);
    assert!(is_detached(&nested));
    assert_eq!(
        gitlink_sha(&root, "vendor/nested"),
        recorded,
        "must not stage"
    );
}

/// `odm.git:own-root` gitlink apply must not inherit the parent repo.
#[test]
fn gitlink_pin_apply_missing_child_repo_is_not_found() {
    let (dir, root) = ws_git_committed();
    add_gitlink_nested(dir.path(), &root);
    let nested = root.join("vendor/nested");
    let parent_before = head_sha(&root);
    let git_entry = nested.join(".git");
    if git_entry.is_dir() {
        fs::remove_dir_all(&git_entry).unwrap();
    } else {
        fs::remove_file(&git_entry).unwrap();
    }
    let root_s = root.to_str().unwrap();

    let out = odm()
        .args(["--root", root_s, "--json", "pin", "apply", "nested"])
        .assert()
        .failure()
        .code(4)
        .get_output()
        .clone();
    let v: Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    assert_eq!(v["error"]["code"], "not_found");
    assert_eq!(head_sha(&root), parent_before, "must not checkout parent");
}
