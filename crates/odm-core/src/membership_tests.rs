use super::*;
use crate::config::{
    save_config, ProjectEntry, ProgenEntry, WorkspaceConfig,
};
use crate::init::{init_workspace, InitOptions};
use crate::paths::{config_path, progen_index_dir};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

use odm_git::Git;

fn git_user(repo: &Path) {
    Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "config", "user.email", "t@est"])
        .status()
        .unwrap();
    Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "config", "user.name", "t"])
        .status()
        .unwrap();
}

fn bare_fixture(root: &Path, name: &str) -> std::path::PathBuf {
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

#[test]
fn project_add_rm_cycle() {
    let dir = tempdir().unwrap();
    let res = init_workspace(InitOptions {
        path: dir.path().to_path_buf(),
        no_git: false,
        name: None,
    })
    .unwrap();
    let root = res.root;
    let bare = bare_fixture(&root, "alpha");
    let g = Git::new();
    let mut cfg = WorkspaceConfig::default();
    project_add(
        &g,
        &root,
        &mut cfg,
        "alpha",
        ProjectEntry {
            path: "projects/alpha".into(),
            url: Some(bare.to_string_lossy().into()),
            branch: Some("main".into()),
            type_: None,
            ..Default::default()
        },
        false,
    )
    .unwrap();
    assert!(g.is_repo(&root.join("projects/alpha")).unwrap());
    assert!(crate::pin::load_pin(&root)
        .unwrap()
        .unwrap()
        .pins
        .contains_key("alpha"));

    project_rm(&g, &root, &mut cfg, "alpha", true, false).unwrap();
    assert!(!cfg.projects.contains_key("alpha"));
    assert!(!root.join("projects/alpha").exists());
}

#[test]
fn progen_rm_strips_group_and_index_dir() {
    let dir = tempdir().unwrap();
    let res = init_workspace(InitOptions {
        path: dir.path().to_path_buf(),
        no_git: true,
        name: None,
    })
    .unwrap();
    let root = res.root;
    let g = Git::new();
    let mut cfg = WorkspaceConfig::default();
    progen_add(
        &g,
        &root,
        &mut cfg,
        "desk",
        ProgenEntry {
            path: "vaults/desk".into(),
            url: None,
            branch: None,
            ..Default::default()
        },
        false,
    )
    .unwrap();
    cfg.progen_groups
        .insert("all".into(), vec!["desk".into(), "other".into()]);
    save_config(&root, &cfg).unwrap();

    let idx = progen_index_dir(&root, "desk");
    fs::create_dir_all(&idx).unwrap();
    fs::write(idx.join("index.db"), b"x").unwrap();
    assert!(idx.exists());

    progen_rm(&g, &root, &mut cfg, "desk", false, false).unwrap();
    assert!(!cfg.progens.contains_key("desk"));
    assert_eq!(
        cfg.progen_groups.get("all").unwrap(),
        &vec!["other".to_string()]
    );
    assert!(!idx.exists());
}

#[test]
fn project_add_rejects_path_escape_without_writing_config() {
    let dir = tempdir().unwrap();
    let res = init_workspace(InitOptions {
        path: dir.path().to_path_buf(),
        no_git: true,
        name: None,
    })
    .unwrap();
    let root = res.root;
    let before = fs::read(config_path(&root)).unwrap();
    let g = Git::new();
    let mut cfg = WorkspaceConfig::default();
    let err = project_add(
        &g,
        &root,
        &mut cfg,
        "evil",
        ProjectEntry {
            path: "../outside".into(),
            url: None,
            branch: None,
            type_: None,
            ..Default::default()
        },
        true,
    )
    .unwrap_err();
    assert!(err.to_string().contains("escape"), "{err}");
    assert!(!cfg.projects.contains_key("evil"));
    assert_eq!(fs::read(config_path(&root)).unwrap(), before);
}

#[test]
fn progen_add_rejects_path_escape_without_writing_config() {
    let dir = tempdir().unwrap();
    let res = init_workspace(InitOptions {
        path: dir.path().to_path_buf(),
        no_git: true,
        name: None,
    })
    .unwrap();
    let root = res.root;
    let before = fs::read(config_path(&root)).unwrap();
    let g = Git::new();
    let mut cfg = WorkspaceConfig::default();
    let err = progen_add(
        &g,
        &root,
        &mut cfg,
        "evil",
        ProgenEntry {
            path: "a/../../outside".into(),
            url: None,
            branch: None,
            ..Default::default()
        },
        true,
    )
    .unwrap_err();
    assert!(err.to_string().contains("escape"), "{err}");
    assert!(!cfg.progens.contains_key("evil"));
    assert_eq!(fs::read(config_path(&root)).unwrap(), before);
}

#[test]
fn project_add_still_rejects_absolute_path() {
    let dir = tempdir().unwrap();
    let res = init_workspace(InitOptions {
        path: dir.path().to_path_buf(),
        no_git: true,
        name: None,
    })
    .unwrap();
    let root = res.root;
    let g = Git::new();
    let mut cfg = WorkspaceConfig::default();
    let err = project_add(
        &g,
        &root,
        &mut cfg,
        "abs",
        ProjectEntry {
            path: "/abs".into(),
            url: None,
            branch: None,
            type_: None,
            ..Default::default()
        },
        true,
    )
    .unwrap_err();
    assert!(err.to_string().contains("relative"), "{err}");
    assert!(!cfg.projects.contains_key("abs"));
}

#[test]
fn membership_add_rejects_cross_map_name() {
    let dir = tempdir().unwrap();
    let res = init_workspace(InitOptions {
        path: dir.path().to_path_buf(),
        no_git: true,
        name: None,
    })
    .unwrap();
    let root = res.root;
    let g = Git::new();
    let mut cfg = WorkspaceConfig::default();
    project_add(
        &g,
        &root,
        &mut cfg,
        "shared",
        ProjectEntry {
            path: "projects/shared".into(),
            url: None,
            branch: None,
            type_: None,
            ..Default::default()
        },
        true,
    )
    .unwrap();
    let err = progen_add(
        &g,
        &root,
        &mut cfg,
        "shared",
        ProgenEntry {
            path: "progens/shared".into(),
            url: None,
            branch: None,
            ..Default::default()
        },
        true,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("already used as a project"),
        "{err}"
    );
    assert!(!cfg.progens.contains_key("shared"));
}

#[test]
fn membership_add_rejects_unsafe_entity_name() {
    let dir = tempdir().unwrap();
    let res = init_workspace(InitOptions {
        path: dir.path().to_path_buf(),
        no_git: true,
        name: None,
    })
    .unwrap();
    let root = res.root;
    let g = Git::new();
    let mut cfg = WorkspaceConfig::default();
    for bad in ["a/b", ".."] {
        let err = project_add(
            &g,
            &root,
            &mut cfg,
            bad,
            ProjectEntry {
                path: "p".into(),
                url: None,
                branch: None,
                type_: None,
                ..Default::default()
            },
            true,
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid project name"), "{err}");
        assert!(!cfg.projects.contains_key(bad));
    }
    project_add(
        &g,
        &root,
        &mut cfg,
        "good-name",
        ProjectEntry {
            path: "projects/good".into(),
            url: None,
            branch: None,
            type_: None,
            ..Default::default()
        },
        true,
    )
    .unwrap();
    assert!(cfg.projects.contains_key("good-name"));
}
