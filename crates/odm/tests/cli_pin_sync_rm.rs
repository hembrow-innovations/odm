//! CLI integration: pin apply --force, named sync, project/progen rm --delete/--force.

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

/// Init workspace + managed project `alpha` cloned from bare fixture.
fn ws_with_project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempdir().unwrap();
    let root = dir.path().join("ws");
    odm()
        .args(["init", root.to_str().unwrap()])
        .assert()
        .success();
    let bare = bare_with_main(dir.path(), "alpha");
    odm()
        .args([
            "--root",
            root.to_str().unwrap(),
            "project",
            "add",
            "alpha",
            "--path",
            "projects/alpha",
            "--url",
            bare.to_str().unwrap(),
            "--branch",
            "main",
        ])
        .assert()
        .success();
    (dir, root)
}

fn json_stdout(cmd: &mut assert_cmd::Command) -> Value {
    let out = cmd.assert().success().get_output().stdout.clone();
    serde_json::from_slice(&out).expect("stdout JSON")
}

// ── pin ──────────────────────────────────────────────────────────────────────

#[test]
fn pin_apply_dirty_exits_3_force_ok() {
    let (_dir, root) = ws_with_project();
    let root_s = root.to_str().unwrap();
    let alpha = root.join("projects/alpha");

    fs::write(alpha.join("dirty"), "x").unwrap();

    odm()
        .args(["--root", root_s, "pin", "apply"])
        .assert()
        .failure()
        .code(3)
        .stderr(predicate::str::contains("dirty"));

    odm()
        .args(["--root", root_s, "pin", "apply", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("applied"));
}

#[test]
fn pin_status_json_stable_fields() {
    let (_dir, root) = ws_with_project();
    let root_s = root.to_str().unwrap();

    let pin = json_stdout(odm().args(["--root", root_s, "--json", "pin", "status"]));
    assert_eq!(pin["present"].as_bool(), Some(true));
    assert!(pin.get("pin_file").and_then(|v| v.as_str()).is_some());
    let entries = pin["entries"].as_array().expect("entries");
    assert!(!entries.is_empty());
    let e = &entries[0];
    assert!(e.get("name").and_then(|v| v.as_str()).is_some());
    assert!(e.get("pin_rev").is_some());
    assert!(e.get("head").is_some());
    assert!(e.get("state").and_then(|v| v.as_str()).is_some());
}

#[test]
fn pin_status_named_subset() {
    let (_dir, root) = ws_with_project();
    let root_s = root.to_str().unwrap();

    let pin = json_stdout(odm().args(["--root", root_s, "--json", "pin", "status", "alpha"]));
    let entries = pin["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["name"], "alpha");
}

// ── sync ─────────────────────────────────────────────────────────────────────

#[test]
fn sync_named_ok_and_json_shape() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("ws");
    odm()
        .args(["init", root.to_str().unwrap()])
        .assert()
        .success();
    let bare = bare_with_main(dir.path(), "alpha");
    // Declare without clone so named sync materializes.
    odm()
        .args([
            "--root",
            root.to_str().unwrap(),
            "project",
            "add",
            "alpha",
            "--path",
            "projects/alpha",
            "--url",
            bare.to_str().unwrap(),
            "--branch",
            "main",
            "--no-clone",
        ])
        .assert()
        .success();
    let root_s = root.to_str().unwrap();

    let v = json_stdout(odm().args(["--root", root_s, "--json", "sync", "alpha"]));
    let results = v["results"].as_array().expect("results");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "alpha");
    assert_eq!(results[0]["fetched"].as_bool(), Some(true));
    assert!(results[0].get("materialized").is_some());
    assert!(results[0].get("head").is_some());
    assert!(root.join("projects/alpha").is_dir());
}

#[test]
fn sync_unknown_name_exits_1() {
    let (_dir, root) = ws_with_project();
    let root_s = root.to_str().unwrap();

    odm()
        .args(["--root", root_s, "sync", "no-such-entity"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("unknown entity"));
}

// ── project rm ───────────────────────────────────────────────────────────────

#[test]
fn project_rm_keeps_tree_by_default() {
    let (_dir, root) = ws_with_project();
    let root_s = root.to_str().unwrap();
    let alpha = root.join("projects/alpha");
    assert!(alpha.is_dir());

    odm()
        .args(["--root", root_s, "project", "rm", "alpha"])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed project alpha"));

    assert!(alpha.is_dir());
    odm()
        .args(["--root", root_s, "project", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("alpha").not());
}

#[test]
fn project_rm_delete_clean_removes_tree() {
    let (_dir, root) = ws_with_project();
    let root_s = root.to_str().unwrap();
    let alpha = root.join("projects/alpha");

    odm()
        .args(["--root", root_s, "project", "rm", "alpha", "--delete"])
        .assert()
        .success();

    assert!(!alpha.exists());
}

#[test]
fn project_rm_delete_dirty_needs_force() {
    let (_dir, root) = ws_with_project();
    let root_s = root.to_str().unwrap();
    let alpha = root.join("projects/alpha");

    fs::write(alpha.join("dirty"), "x").unwrap();

    odm()
        .args(["--root", root_s, "project", "rm", "alpha", "--delete"])
        .assert()
        .failure()
        .code(3)
        .stderr(predicate::str::contains("dirty"));

    assert!(alpha.is_dir());

    odm()
        .args([
            "--root", root_s, "project", "rm", "alpha", "--delete", "--force",
        ])
        .assert()
        .success();

    assert!(!alpha.exists());
}

// ── progen rm ────────────────────────────────────────────────────────────────

#[test]
fn progen_rm_undeclares_without_delete() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("ws");
    odm()
        .args(["init", root.to_str().unwrap(), "--no-git"])
        .assert()
        .success();
    let root_s = root.to_str().unwrap();

    odm()
        .args([
            "--root",
            root_s,
            "progen",
            "add",
            "desk",
            "--path",
            "vaults/desk",
        ])
        .assert()
        .success();
    let vault = root.join("vaults/desk");
    assert!(vault.is_dir());

    odm()
        .args(["--root", root_s, "progen", "rm", "desk"])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed progen desk"));

    assert!(vault.is_dir());
    odm()
        .args(["--root", root_s, "progen", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("desk").not());
}

#[test]
fn progen_rm_delete_removes_clean_path() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("ws");
    odm()
        .args(["init", root.to_str().unwrap(), "--no-git"])
        .assert()
        .success();
    let root_s = root.to_str().unwrap();

    odm()
        .args([
            "--root",
            root_s,
            "progen",
            "add",
            "desk",
            "--path",
            "vaults/desk",
        ])
        .assert()
        .success();
    let vault = root.join("vaults/desk");
    assert!(vault.is_dir());

    odm()
        .args(["--root", root_s, "progen", "rm", "desk", "--delete"])
        .assert()
        .success();

    assert!(!vault.exists());
}

#[test]
fn progen_rm_unknown_exits_1() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("ws");
    odm()
        .args(["init", root.to_str().unwrap(), "--no-git"])
        .assert()
        .success();

    odm()
        .args([
            "--root",
            root.to_str().unwrap(),
            "progen",
            "rm",
            "no-such-progen",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("unknown progen"));
}

// ── gitlink add / rm (odm.git:gitlink-opt-in, odm.git:managed-url, odm.git:rm-keep, odm.git:gitlink-pin-index) ──

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
    String::from_utf8(out.stdout).unwrap().trim().to_string()
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

/// `odm.git:gitlink-opt-in` `odm.git:gitlink-pin-index` `odm.cli:json`
#[test]
fn gitlink_add() {
    let (dir, root) = ws_git_committed();
    let bare = bare_with_main(dir.path(), "nested");
    let root_s = root.to_str().unwrap();

    let v = json_stdout(odm_file_protocol().args([
        "--root",
        root_s,
        "--json",
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
    ]));
    assert_eq!(v["ok"], true);
    assert_eq!(v["name"], "nested");
    assert_eq!(v["materialized"], "gitlink_added");

    assert!(root.join("vendor/nested").exists());
    let cfg = fs::read_to_string(root.join(".odm/odm.config.yaml")).unwrap();
    assert!(cfg.contains("checkout: gitlink"), "{cfg}");
    let gi = fs::read_to_string(root.join(".gitignore")).unwrap_or_default();
    assert!(
        !gi.contains("vendor/nested"),
        "gitignore must skip gitlink path: {gi}"
    );
    let lock = root.join(".odm/odm.lock.yaml");
    assert!(
        !lock.exists() || !fs::read_to_string(&lock).unwrap().contains("nested"),
        "pin file must not list gitlink name"
    );
    assert!(
        stage_row(&root, "vendor/nested").contains("160000"),
        "gitlink should be staged"
    );
}

/// `odm.git:gitlink-opt-in`
#[test]
fn gitlink_add_progen() {
    let (dir, root) = ws_git_committed();
    let bare = bare_with_main(dir.path(), "docs");
    let root_s = root.to_str().unwrap();

    let v = json_stdout(odm_file_protocol().args([
        "--root",
        root_s,
        "--json",
        "progen",
        "add",
        "docs",
        "--path",
        "vendor/docs",
        "--url",
        bare.to_str().unwrap(),
        "--branch",
        "main",
        "--gitlink",
    ]));
    assert_eq!(v["materialized"], "gitlink_added");
    let cfg = fs::read_to_string(root.join(".odm/odm.config.yaml")).unwrap();
    assert!(cfg.contains("checkout: gitlink"), "{cfg}");
}

/// `odm.git:managed-url` `odm.cli:exit-codes`
#[test]
fn gitlink_requires_url_cli() {
    odm()
        .args([
            "project",
            "add",
            "nested",
            "--path",
            "vendor/nested",
            "--gitlink",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("url"));
    odm()
        .args([
            "progen",
            "add",
            "docs",
            "--path",
            "vendor/docs",
            "--gitlink",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("url"));
}

/// `odm.git:rm-keep` `odm.git:gitlink-pin-index`
#[test]
fn gitlink_rm_keeps_tree() {
    let (dir, root) = ws_git_committed();
    let bare = bare_with_main(dir.path(), "nested");
    let root_s = root.to_str().unwrap();

    odm_file_protocol()
        .args([
            "--root",
            root_s,
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
    let nested = root.join("vendor/nested");
    assert!(nested.exists());
    let before = head_sha(&root);

    odm()
        .args(["--root", root_s, "project", "rm", "nested"])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed project nested"));

    assert!(nested.exists(), "rm keeps the tree");
    assert_eq!(head_sha(&root), before, "rm must not commit workspace root");
    assert!(
        !stage_row(&root, "vendor/nested").contains("160000"),
        "rm must unstage the gitlink"
    );
    odm()
        .args(["--root", root_s, "project", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("nested").not());
    let lock = root.join(".odm/odm.lock.yaml");
    assert!(
        !lock.exists() || !fs::read_to_string(&lock).unwrap().contains("nested"),
        "pin file must not list gitlink name"
    );
}

fn gitlink_sha(repo: &Path, rel: &str) -> String {
    stage_row(repo, rel)
        .split_whitespace()
        .nth(1)
        .expect("gitlink sha")
        .to_ascii_lowercase()
}

fn rev_parse(repo: &Path, rev: &str) -> String {
    let out = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "rev-parse", rev])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .trim()
        .to_ascii_lowercase()
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

/// `odm.git:sync-fetch-only` `odm.cli:sync` `odm.git:gitlink-pin-index`
#[test]
fn gitlink_sync_fetch_only() {
    let (dir, root) = ws_git_committed();
    add_gitlink_nested(dir.path(), &root);
    let root_s = root.to_str().unwrap();
    let nested = root.join("vendor/nested");
    let child_before = head_sha(&nested);
    let link_before = gitlink_sha(&root, "vendor/nested");
    assert_eq!(child_before, link_before);

    let seed = dir.path().join("nested-seed");
    fs::write(seed.join("MORE"), "x").unwrap();
    assert!(Command::new("git")
        .args(["-C", seed.to_str().unwrap(), "add", "MORE"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["-C", seed.to_str().unwrap(), "commit", "-m", "more"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["-C", seed.to_str().unwrap(), "push", "origin", "main"])
        .status()
        .unwrap()
        .success());
    let remote_head = head_sha(&seed);
    assert_ne!(remote_head, child_before);

    let v = json_stdout(odm_file_protocol().args(["--root", root_s, "--json", "sync", "nested"]));
    let results = v["results"].as_array().expect("results");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "nested");
    assert_eq!(results[0]["fetched"].as_bool(), Some(true));

    assert_eq!(head_sha(&nested), child_before, "child HEAD must stay put");
    assert_eq!(
        gitlink_sha(&root, "vendor/nested"),
        link_before,
        "parent gitlink SHA must stay put"
    );
    assert!(!nested.join("MORE").exists(), "fetch must not checkout");
    assert_eq!(rev_parse(&nested, "origin/main"), remote_head);
    let lock = root.join(".odm/odm.lock.yaml");
    assert!(
        !lock.exists() || !fs::read_to_string(&lock).unwrap().contains("nested"),
        "lock file must not list gitlink name"
    );
}

/// `odm.git:gitlink-pin-index` `odm.cli:pin-apply` `odm.git:project-git`
#[test]
fn gitlink_status_in_sync() {
    let (dir, root) = ws_git_committed();
    add_gitlink_nested(dir.path(), &root);
    let root_s = root.to_str().unwrap();
    let nested = root.join("vendor/nested");
    let link_sha = gitlink_sha(&root, "vendor/nested");
    let child_head = head_sha(&nested);
    assert_eq!(link_sha, child_head);

    let st = json_stdout(odm().args(["--root", root_s, "--json", "status"]));
    let p = st["projects"]
        .as_array()
        .expect("projects")
        .iter()
        .find(|e| e["name"] == "nested")
        .expect("nested");
    assert_eq!(p["pin_state"], "in_sync");
    assert_eq!(p["pin_rev"].as_str().unwrap(), link_sha);
    assert_eq!(p["head"].as_str().unwrap(), child_head);
    assert_eq!(p["dirty"].as_bool(), Some(false));

    let fake = "b".repeat(40);
    fs::write(
        root.join(".odm/odm.lock.yaml"),
        format!(
            "version: 1\npins:\n  nested:\n    rev: {fake}\n    url: https://example.com/nested.git\n"
        ),
    )
    .unwrap();
    let st = json_stdout(odm().args(["--root", root_s, "--json", "status"]));
    let p = st["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "nested")
        .unwrap();
    assert_eq!(p["pin_rev"].as_str().unwrap(), link_sha);
    assert_eq!(p["pin_state"], "in_sync");
    fs::remove_file(root.join(".odm/odm.lock.yaml")).unwrap();

    fs::write(nested.join("dirty"), "x").unwrap();
    let st = json_stdout(odm().args(["--root", root_s, "--json", "status"]));
    let p = st["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "nested")
        .unwrap();
    assert_eq!(p["dirty"].as_bool(), Some(true));
    assert_eq!(p["pin_state"], "in_sync");
    assert_eq!(p["head"].as_str().unwrap(), child_head);

    git_user(&nested);
    odm()
        .args([
            "--root", root_s, "project", "git", "nested", "--", "add", "dirty",
        ])
        .assert()
        .success();
    odm()
        .args([
            "--root", root_s, "project", "git", "nested", "--", "commit", "-m", "dirty",
        ])
        .assert()
        .success();
    let lock = root.join(".odm/odm.lock.yaml");
    assert!(
        !lock.exists() || !fs::read_to_string(&lock).unwrap().contains("nested"),
        "project git auto-maintain skips gitlink"
    );
}
