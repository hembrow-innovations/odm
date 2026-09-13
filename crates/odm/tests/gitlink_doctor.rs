//! CLI: doctor gitlink extras, missing, conflict, checkout mismatch, gitmodules layout.
//! `odm.git:doctor-gitlink` `odm.cli:status-doctor` `odm.layout:config-truth`

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use assert_cmd::cargo::cargo_bin;
use serde_json::Value;
use tempfile::tempdir;

fn odm() -> assert_cmd::Command {
    assert_cmd::Command::new(cargo_bin("odm"))
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

fn json_code(cmd: &mut assert_cmd::Command, code: i32) -> Value {
    let out = cmd.assert().code(code).get_output().stdout.clone();
    serde_json::from_slice(&out).expect("stdout JSON")
}

fn check<'a>(doc: &'a Value, id: &str) -> &'a Value {
    doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("missing check {id} in {doc}"))
}

fn stage_gitlink(repo: &Path, rel: &str, sha: &str) {
    let spec = format!("160000,{sha},{rel}");
    assert!(Command::new("git")
        .args([
            "-C",
            repo.to_str().unwrap(),
            "update-index",
            "--add",
            "--cacheinfo",
            &spec,
        ])
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
    child
        .stdin
        .take()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}

fn head_sha(repo: &Path) -> String {
    let out = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn origin_url(repo: &Path) -> String {
    let out = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "remote", "get-url", "origin"])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn declare_gitlink(root: &Path, url: &str) {
    fs::write(
        root.join(".odm/odm.config.yaml"),
        format!(
            "projects:\n  nested:\n    path: vendor/nested\n    url: {url}\n    branch: main\n    checkout: gitlink\n"
        ),
    )
    .unwrap();
}

/// `odm.git:doctor-gitlink` `odm.layout:config-truth`
#[test]
fn gitlink_doctor_extra_fails_and_is_not_imported() {
    let (_dir, root) = ws_git_committed();
    let root_s = root.to_str().unwrap();
    stage_gitlink(
        &root,
        "vendor/orphan",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );

    let doc = json_code(odm().args(["--root", root_s, "--json", "doctor"]), 3);
    assert_eq!(doc["ok"], false);
    let extra = check(&doc, "gitlink_extra");
    assert_eq!(extra["status"], "fail");
    assert_eq!(extra["fixable"], false);
    assert!(
        extra["message"].as_str().unwrap().contains("vendor/orphan"),
        "{}",
        extra["message"]
    );

    let cfg_before = fs::read_to_string(root.join(".odm/odm.config.yaml")).unwrap();
    let doc = json_code(
        odm().args(["--root", root_s, "--json", "doctor", "--fix"]),
        3,
    );
    assert_eq!(check(&doc, "gitlink_extra")["status"], "fail");
    let cfg_after = fs::read_to_string(root.join(".odm/odm.config.yaml")).unwrap();
    assert_eq!(cfg_before, cfg_after, "extras must not be imported as projects");
    let listed = odm()
        .args(["--root", root_s, "project", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(
        !String::from_utf8_lossy(&listed).contains("orphan"),
        "extra gitlink must not become a project"
    );
}

/// `odm.git:doctor-gitlink`
#[test]
fn gitlink_doctor_missing_fails() {
    let (dir, root) = ws_git_committed();
    let bare = bare_with_main(dir.path(), "nested");
    declare_gitlink(&root, bare.to_str().unwrap());
    fs::create_dir_all(root.join("vendor/nested")).unwrap();
    let root_s = root.to_str().unwrap();

    let doc = json_code(odm().args(["--root", root_s, "--json", "doctor"]), 3);
    assert_eq!(doc["ok"], false);
    let missing = check(&doc, "gitlink_missing");
    assert_eq!(missing["status"], "fail");
    assert_eq!(missing["fixable"], false);
    assert!(
        missing["message"].as_str().unwrap().contains("nested"),
        "{}",
        missing["message"]
    );
}

/// `odm.git:doctor-gitlink`
#[test]
fn gitlink_doctor_conflict_fails() {
    let (dir, root) = ws_git_committed();
    add_gitlink_nested(dir.path(), &root);
    stage_gitlink_conflict(
        &root,
        "vendor/nested",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );
    let root_s = root.to_str().unwrap();

    let doc = json_code(odm().args(["--root", root_s, "--json", "doctor"]), 3);
    assert_eq!(doc["ok"], false);
    let conflict = check(&doc, "gitlink_conflict");
    assert_eq!(conflict["status"], "fail");
    assert_eq!(conflict["fixable"], false);
}

/// `odm.git:doctor-gitlink` `odm.git:gitlink-opt-in`
#[test]
fn gitlink_doctor_checkout_mismatch_fails() {
    let (dir, root) = ws_git_committed();
    let bare = bare_with_main(dir.path(), "nested");
    fs::write(
        root.join(".odm/odm.config.yaml"),
        format!(
            "projects:\n  nested:\n    path: vendor/nested\n    url: {}\n    branch: main\n",
            bare.to_str().unwrap()
        ),
    )
    .unwrap();
    stage_gitlink(
        &root,
        "vendor/nested",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    let root_s = root.to_str().unwrap();

    let doc = json_code(odm().args(["--root", root_s, "--json", "doctor"]), 3);
    assert_eq!(doc["ok"], false);
    let mismatch = check(&doc, "checkout_mismatch");
    assert_eq!(mismatch["status"], "fail");
    assert_eq!(mismatch["fixable"], false);
    assert_eq!(check(&doc, "gitlink_extra")["status"], "pass");
}

/// `odm.git:doctor-gitlink`
#[test]
fn gitlink_doctor_checkout_mismatch_both_managed_and_gitlink() {
    let (dir, root) = ws_git_committed();
    add_gitlink_nested(dir.path(), &root);
    let nested = root.join("vendor/nested");
    let gitfile = nested.join(".git");
    if gitfile.is_file() {
        fs::remove_file(&gitfile).unwrap();
    } else if gitfile.is_dir() {
        fs::remove_dir_all(&gitfile).unwrap();
    }
    assert!(Command::new("git")
        .args(["init", nested.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    assert!(nested.join(".git").is_dir());
    let root_s = root.to_str().unwrap();

    let doc = json_code(odm().args(["--root", root_s, "--json", "doctor"]), 3);
    assert_eq!(doc["ok"], false);
    let mismatch = check(&doc, "checkout_mismatch");
    assert_eq!(mismatch["status"], "fail");
}

/// `odm.git:doctor-gitlink` `odm.cli:status-doctor`
#[test]
fn gitlink_doctor_gitmodules_layout_fails_and_fix_rewrites() {
    let (dir, root) = ws_git_committed();
    add_gitlink_nested(dir.path(), &root);
    fs::write(root.join(".gitmodules"), "[submodule \"stale\"]\n").unwrap();
    let root_s = root.to_str().unwrap();

    let doc = json_code(odm().args(["--root", root_s, "--json", "doctor"]), 3);
    assert_eq!(doc["ok"], false);
    let layout = check(&doc, "gitmodules_layout");
    assert_eq!(layout["status"], "fail");
    assert_eq!(layout["fixable"], true);

    let doc = json_code(odm().args(["--root", root_s, "--json", "doctor", "--fix"]), 0);
    assert_eq!(doc["ok"], true);
    assert_eq!(check(&doc, "gitmodules_layout")["status"], "pass");
    let text = fs::read_to_string(root.join(".gitmodules")).unwrap();
    assert!(text.contains("[submodule \"vendor/nested\"]"), "{text}");
    assert!(text.contains("path = vendor/nested"), "{text}");
}

/// `odm.git:doctor-gitlink` `odm.cli:status-doctor`
#[test]
fn gitlink_doctor_fix_does_not_rewrite_remotes_or_pin_apply() {
    let (dir, root) = ws_git_committed();
    add_gitlink_nested(dir.path(), &root);
    let nested = root.join("vendor/nested");
    let child_before = head_sha(&nested);
    let origin_before = origin_url(&nested);
    assert!(Command::new("git")
        .args([
            "-C",
            nested.to_str().unwrap(),
            "remote",
            "set-url",
            "origin",
            "https://example.com/other.git",
        ])
        .status()
        .unwrap()
        .success());

    let root_s = root.to_str().unwrap();
    let doc = json_code(
        odm().args(["--root", root_s, "--json", "doctor", "--fix"]),
        3,
    );
    assert_eq!(doc["ok"], false);
    assert_eq!(
        check(&doc, "origin_match:project:nested")["status"],
        "fail"
    );
    assert_eq!(origin_url(&nested), "https://example.com/other.git");
    assert_ne!(origin_url(&nested), origin_before);
    assert_eq!(head_sha(&nested), child_before, "doctor --fix must not pin apply");
    let lock = root.join(".odm/odm.lock.yaml");
    assert!(
        !lock.exists() || !fs::read_to_string(&lock).unwrap().contains("nested"),
        "doctor --fix must not write gitlink names into the pin file"
    );
}
