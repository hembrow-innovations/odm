//! CLI: `odm pin record` stages gitlink child HEAD. `odm.git:gitlink-pin-index` `odm.cli:json` `odm.cli:exit-codes`

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

fn json_stdout(cmd: &mut assert_cmd::Command) -> Value {
    let out = cmd.assert().success().get_output().stdout.clone();
    serde_json::from_slice(&out).expect("stdout JSON")
}

/// `odm.git:gitlink-pin-index` `odm.cli:exit-codes`
#[test]
fn gitlink_pin_record_refuses_dirty() {
    let (dir, root) = ws_git_committed();
    add_gitlink_nested(dir.path(), &root);
    let root_s = root.to_str().unwrap();
    let nested = root.join("vendor/nested");
    let parent_before = head_sha(&root);
    let link_before = gitlink_sha(&root, "vendor/nested");
    fs::write(nested.join("dirty"), "x").unwrap();

    odm()
        .args(["--root", root_s, "pin", "record"])
        .assert()
        .failure()
        .code(3)
        .stderr(predicate::str::contains("dirty"));

    assert_eq!(gitlink_sha(&root, "vendor/nested"), link_before);
    assert_eq!(head_sha(&root), parent_before);

    odm()
        .args(["--root", root_s, "pin", "record", "--force"])
        .assert()
        .success();

    assert_eq!(gitlink_sha(&root, "vendor/nested"), head_sha(&nested));
    assert_eq!(head_sha(&root), parent_before, "must not commit workspace root");
}

/// `odm.git:gitlink-pin-index` `odm.cli:json` `odm.cli:exit-codes`
#[test]
fn gitlink_pin_record_stages() {
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
    let parent_before = head_sha(&root);
    let link_before = gitlink_sha(&root, "vendor/nested");
    commit_child(&nested, "MORE", "x");
    let child_head = head_sha(&nested);
    assert_ne!(child_head, link_before);

    odm()
        .args(["--root", root_s, "pin", "record", "alpha"])
        .assert()
        .failure()
        .code(1);

    let out = odm()
        .args(["--root", root_s, "--json", "pin", "record"])
        .assert()
        .success()
        .stderr(predicate::str::contains("commit"))
        .get_output()
        .clone();
    let v: Value = serde_json::from_slice(&out.stdout).expect("stdout JSON");
    let results = v["results"].as_array().expect("results");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "nested");
    assert_eq!(results[0]["status"], "recorded");
    assert_eq!(results[0]["rev"].as_str().unwrap(), child_head);
    assert!(results[0].get("detached").is_none());

    assert_eq!(gitlink_sha(&root, "vendor/nested"), child_head);
    assert_eq!(head_sha(&root), parent_before, "must not commit workspace root");
    let lock = root.join(".odm/odm.lock.yaml");
    assert!(
        !lock.exists() || !fs::read_to_string(&lock).unwrap().contains("nested"),
        "pin file must not list gitlink name"
    );

    let again = json_stdout(odm().args(["--root", root_s, "--json", "pin", "record", "nested"]));
    assert_eq!(again["results"][0]["rev"].as_str().unwrap(), child_head);
    assert_eq!(head_sha(&root), parent_before);
    assert_eq!(gitlink_sha(&root, "vendor/nested"), child_head);
}
