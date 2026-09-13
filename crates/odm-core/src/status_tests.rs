use super::*;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::{Arc, Mutex};

use odm_git::{CommandOutput, CommandRunner, Git};
use tempfile::tempdir;

use crate::config::{ProjectEntry, ProgenEntry, WorkspaceConfig};
use crate::paths::worktree_slot_path;

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
    out_fail("not a repo")
}

fn sha40() -> String {
    "a".repeat(40)
}

/// Observation git sample for one on-disk git entity: is_repo, head, origin, clean.
fn observe_git_ok() -> Vec<CommandOutput> {
    vec![
        is_repo_true(),
        out_ok_stdout(&format!("{}\n", sha40())),
        out_ok_stdout("https://example.com/a.git\n"),
        out_ok(), // clean
    ]
}

/// Primary checkout dir with a `.git` marker so [`Git::is_repo_root`] runs the
/// scripted `is_repo` probe (own checkout, not ancestor-only).
fn ensure_primary(root: &Path, rel: &str) -> PathBuf {
    let p = root.join(rel);
    fs::create_dir_all(&p).unwrap();
    fs::write(p.join(".git"), "gitdir: mock\n").unwrap();
    p
}

fn ensure_primary_plain(root: &Path, rel: &str) -> PathBuf {
    let p = root.join(rel);
    fs::create_dir_all(&p).unwrap();
    p
}

fn ws_project_and_progen(root: PathBuf) -> Workspace {
    let mut projects = BTreeMap::new();
    projects.insert(
        "alpha".into(),
        ProjectEntry {
            path: "projects/alpha".into(),
            url: None,
            branch: None,
            type_: None,
            ..Default::default()
        },
    );
    let mut progens = BTreeMap::new();
    progens.insert(
        "notes".into(),
        ProgenEntry {
            path: "progens/notes".into(),
            url: None,
            branch: None,
            ..Default::default()
        },
    );
    Workspace {
        root,
        config: WorkspaceConfig {
            projects,
            progens,
            ..Default::default()
        },
        actions: BTreeMap::new(),
        generators: BTreeMap::new(),
    }
}

#[test]
fn pin_source_classifies_checkout_mode() {
    use crate::config::CheckoutMode;
    assert_eq!(pin_source(false, CheckoutMode::Clone), PinSource::Unmanaged);
    assert_eq!(
        pin_source(false, CheckoutMode::Gitlink),
        PinSource::Unmanaged
    );
    assert_eq!(pin_source(true, CheckoutMode::Clone), PinSource::LockFile);
    assert_eq!(pin_source(true, CheckoutMode::Gitlink), PinSource::Gitlink);
}

#[test]
fn pin_source_gitlink_never_missing_pin_file() {
    let sha = "a".repeat(40);
    assert_ne!(
        compute_pin_state(PinSource::Gitlink, false, true, None, Some(&sha)),
        PinState::MissingPinFile
    );
    assert_ne!(
        compute_pin_state(PinSource::Gitlink, false, false, None, None),
        PinState::MissingPinFile
    );
    assert_eq!(
        compute_pin_state(PinSource::Gitlink, false, true, None, Some(&sha)),
        PinState::Unpinned
    );
    assert_eq!(
        compute_pin_state(PinSource::Gitlink, true, true, Some(&sha), Some(&sha)),
        PinState::InSync
    );
    assert_eq!(
        compute_pin_state(PinSource::Gitlink, false, true, Some(&sha), Some("b")),
        PinState::Drift
    );
    assert_eq!(
        compute_pin_state(PinSource::Gitlink, false, true, Some(&sha), None),
        PinState::Drift
    );
    assert_eq!(
        compute_pin_state(PinSource::Gitlink, false, false, Some(&sha), None),
        PinState::MissingPath
    );
}

#[test]
fn pin_state_matrix() {
    // unmanaged
    assert_eq!(
        compute_pin_state(PinSource::Unmanaged, false, true, None, None),
        PinState::None
    );
    assert_eq!(
        compute_pin_state(PinSource::Unmanaged, true, false, Some("a"), None),
        PinState::None
    );
    // missing pin file
    assert_eq!(
        compute_pin_state(PinSource::LockFile, false, true, None, Some("a")),
        PinState::MissingPinFile
    );
    assert_eq!(
        compute_pin_state(PinSource::LockFile, false, false, None, None),
        PinState::MissingPinFile
    );
    // unpinned (pin present, no entry) — including path missing
    assert_eq!(
        compute_pin_state(PinSource::LockFile, true, true, None, Some("a")),
        PinState::Unpinned
    );
    assert_eq!(
        compute_pin_state(PinSource::LockFile, true, false, None, None),
        PinState::Unpinned
    );
    // missing path (pin entry, not on disk)
    assert_eq!(
        compute_pin_state(PinSource::LockFile, true, false, Some("a"), None),
        PinState::MissingPath
    );
    let sha = "a".repeat(40);
    // in_sync
    assert_eq!(
        compute_pin_state(PinSource::LockFile, true, true, Some(&sha), Some(&sha)),
        PinState::InSync
    );
    // drift: head differs
    assert_eq!(
        compute_pin_state(PinSource::LockFile, true, true, Some(&sha), Some("b")),
        PinState::Drift
    );
    // drift: on disk but not git / no head (former lifecycle "missing_path")
    assert_eq!(
        compute_pin_state(PinSource::LockFile, true, true, Some(&sha), None),
        PinState::Drift
    );
    assert_eq!(PinState::InSync.as_str(), "in_sync");
    assert_eq!(PinState::MissingPinFile.as_str(), "missing_pin_file");
}

#[test]
fn build_status_includes_registered_worktree_slots() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    fs::create_dir_all(root.join("progens/notes")).unwrap();
    let ws = ws_project_and_progen(root.to_path_buf());
    let s_b = worktree_slot_path(root, "alpha", "b-slot");
    let s_a = worktree_slot_path(root, "alpha", "a-slot");
    let porcelain = format!(
        "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n\
         worktree {}\nHEAD def\nbranch refs/heads/b\n\n\
         worktree {}\nHEAD ghi\nbranch refs/heads/a\n\n",
        primary.display(),
        s_b.display(),
        s_a.display(),
    );
    let mut outs = observe_git_ok();
    // path-only progen: no .git → is_repo_root short-circuits (no scripted call)
    // worktree_list for project
    outs.push(is_repo_true());
    outs.push(out_ok_stdout(&porcelain));
    let (runner, _) = ScriptedRunner::new(outs);
    let g = Git::with_runner(runner);
    let snap = build_status(&g, &ws).unwrap();
    let p = &snap.projects[0];
    let slots = p.worktree_slots.as_ref().expect("project slots");
    assert_eq!(slots.len(), 2);
    assert!(!snap.progens[0].is_git);
    assert_eq!(slots[0].name, "a-slot");
    assert_eq!(slots[0].path, "worktrees/alpha/a-slot");
    // ScriptedRunner empty queue → is_clean ok/empty → dirty false
    assert_eq!(slots[0].dirty, Some(false));
    assert_eq!(slots[1].name, "b-slot");
    assert_eq!(slots[1].path, "worktrees/alpha/b-slot");
    assert_eq!(slots[1].dirty, Some(false));
    let v = serde_json::to_value(&snap).unwrap();
    assert!(v["projects"][0]["worktree_slots"].is_array());
    assert_eq!(v["projects"][0]["worktree_slots"][0]["name"], "a-slot");
    assert_eq!(v["projects"][0]["worktree_slots"][0]["dirty"], false);
    assert!(v["progens"][0].get("worktree_slots").is_none());
    assert!(v["progens"][0].get("worktree_orphans").is_none());
    // no orphan dirs on disk → empty array present
    assert_eq!(
        p.worktree_orphans.as_ref().expect("project orphans").as_slice(),
        &[]
    );
    assert_eq!(v["projects"][0]["worktree_orphans"], serde_json::json!([]));
}

#[test]
fn build_status_lists_orphan_slot_dirs() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    fs::create_dir_all(root.join("progens/notes")).unwrap();
    let ws = ws_project_and_progen(root.to_path_buf());
    let s_reg = worktree_slot_path(root, "alpha", "keep");
    let s_orphan = worktree_slot_path(root, "alpha", "stale");
    fs::create_dir_all(&s_orphan).unwrap();
    // also a second orphan to prove sort
    fs::create_dir_all(worktree_slot_path(root, "alpha", "other")).unwrap();
    // file under worktrees prefix ignored (not a dir)
    fs::write(root.join("worktrees/alpha/not-a-dir"), b"x").unwrap();
    let porcelain = format!(
        "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n\
         worktree {}\nHEAD def\nbranch refs/heads/k\n\n",
        primary.display(),
        s_reg.display(),
    );
    let mut outs = observe_git_ok();
    // path-only progen: no .git marker → no observe git call
    outs.push(is_repo_true());
    outs.push(out_ok_stdout(&porcelain));
    let (runner, _) = ScriptedRunner::new(outs);
    let g = Git::with_runner(runner);
    let snap = build_status(&g, &ws).unwrap();
    let p = &snap.projects[0];
    let slots = p.worktree_slots.as_ref().expect("slots");
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].name, "keep");
    let orphans = p.worktree_orphans.as_ref().expect("orphans");
    assert_eq!(orphans.len(), 2);
    assert_eq!(orphans[0].name, "other");
    assert_eq!(orphans[0].path, "worktrees/alpha/other");
    assert_eq!(orphans[1].name, "stale");
    assert_eq!(orphans[1].path, "worktrees/alpha/stale");
    // registered slot must not appear as orphan
    assert!(orphans.iter().all(|o| o.name != "keep"));
    let v = serde_json::to_value(&snap).unwrap();
    assert_eq!(
        v["projects"][0]["worktree_orphans"],
        serde_json::json!([
            {"name": "other", "path": "worktrees/alpha/other"},
            {"name": "stale", "path": "worktrees/alpha/stale"}
        ])
    );
    // no dirty key on orphans
    assert!(v["projects"][0]["worktree_orphans"][0].get("dirty").is_none());
    assert!(v["progens"][0].get("worktree_orphans").is_none());
}

#[test]
fn build_status_empty_orphans_when_missing_worktrees_dir() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let primary = ensure_primary(root, "projects/alpha");
    let mut projects = BTreeMap::new();
    projects.insert(
        "alpha".into(),
        ProjectEntry {
            path: "projects/alpha".into(),
            url: None,
            branch: None,
            type_: None,
            ..Default::default()
        },
    );
    let ws = Workspace {
        root: root.to_path_buf(),
        config: WorkspaceConfig {
            projects,
            ..Default::default()
        },
        actions: BTreeMap::new(),
        generators: BTreeMap::new(),
    };
    let porcelain = format!(
        "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n",
        primary.display(),
    );
    let mut outs = observe_git_ok();
    outs.push(is_repo_true());
    outs.push(out_ok_stdout(&porcelain));
    let (runner, _) = ScriptedRunner::new(outs);
    let g = Git::with_runner(runner);
    let snap = build_status(&g, &ws).unwrap();
    assert_eq!(
        snap.projects[0]
            .worktree_orphans
            .as_ref()
            .unwrap()
            .as_slice(),
        &[]
    );
}

#[test]
fn build_status_empty_slots_when_list_errors_or_non_git() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    // plain dir (no .git): observe short-circuits; list still probes is_repo
    ensure_primary_plain(root, "projects/alpha");
    let mut projects = BTreeMap::new();
    projects.insert(
        "alpha".into(),
        ProjectEntry {
            path: "projects/alpha".into(),
            url: None,
            branch: None,
            type_: None,
            ..Default::default()
        },
    );
    let ws = Workspace {
        root: root.to_path_buf(),
        config: WorkspaceConfig {
            projects,
            ..Default::default()
        },
        actions: BTreeMap::new(),
        generators: BTreeMap::new(),
    };
    // non-git primary: observe no call; list is_repo false → Err → []
    // orphan dir on disk must not surface when list soft-fails
    fs::create_dir_all(worktree_slot_path(root, "alpha", "stale")).unwrap();
    let (runner, _) = ScriptedRunner::new(vec![is_repo_false()]);
    let g = Git::with_runner(runner);
    let snap = build_status(&g, &ws).unwrap();
    assert!(!snap.projects[0].is_git);
    assert_eq!(snap.projects[0].worktree_slots.as_ref().unwrap().len(), 0);
    assert_eq!(snap.projects[0].worktree_orphans.as_ref().unwrap().len(), 0);
    let v = serde_json::to_value(&snap).unwrap();
    assert_eq!(v["projects"][0]["worktree_slots"], serde_json::json!([]));
    assert_eq!(v["projects"][0]["worktree_orphans"], serde_json::json!([]));
}

#[test]
fn build_status_soft_fails_worktree_list_error() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    ensure_primary(root, "projects/alpha");
    fs::create_dir_all(worktree_slot_path(root, "alpha", "stale")).unwrap();
    let mut projects = BTreeMap::new();
    projects.insert(
        "alpha".into(),
        ProjectEntry {
            path: "projects/alpha".into(),
            url: None,
            branch: None,
            type_: None,
            ..Default::default()
        },
    );
    let ws = Workspace {
        root: root.to_path_buf(),
        config: WorkspaceConfig {
            projects,
            ..Default::default()
        },
        actions: BTreeMap::new(),
        generators: BTreeMap::new(),
    };
    let mut outs = observe_git_ok();
    outs.push(is_repo_true());
    outs.push(out_fail("worktree list boom"));
    let (runner, _) = ScriptedRunner::new(outs);
    let g = Git::with_runner(runner);
    let snap = build_status(&g, &ws).unwrap();
    assert_eq!(
        snap.projects[0].worktree_slots.as_ref().unwrap().as_slice(),
        &[]
    );
    assert_eq!(
        snap.projects[0]
            .worktree_orphans
            .as_ref()
            .unwrap()
            .as_slice(),
        &[]
    );
}

#[test]
fn format_status_human_shows_slots_when_non_empty() {
    let snap = StatusSnapshot {
        root: "/ws".into(),
        projects: vec![EntityStatus {
            name: "alpha".into(),
            path: "projects/alpha".into(),
            url: None,
            managed: false,
            on_disk: true,
            is_git: true,
            head: None,
            pin_rev: None,
            pin_state: PinState::None,
            dirty: Some(false),
            worktree_slots: Some(vec![
                WorktreeSlotInfo {
                    name: "a".into(),
                    path: "worktrees/alpha/a".into(),
                    dirty: Some(false),
                },
                WorktreeSlotInfo {
                    name: "b".into(),
                    path: "worktrees/alpha/b".into(),
                    dirty: Some(true),
                },
            ]),
            worktree_orphans: Some(vec![]),
        }],
        progens: vec![],
    };
    let human = format_status_human(&snap);
    assert!(human.contains("worktrees: a, b dirty"), "{human}");
}

#[test]
fn format_status_human_silent_when_slots_empty() {
    let snap = StatusSnapshot {
        root: "/ws".into(),
        projects: vec![EntityStatus {
            name: "alpha".into(),
            path: "projects/alpha".into(),
            url: None,
            managed: false,
            on_disk: true,
            is_git: true,
            head: None,
            pin_rev: None,
            pin_state: PinState::None,
            dirty: None,
            worktree_slots: Some(vec![]),
            worktree_orphans: Some(vec![]),
        }],
        progens: vec![EntityStatus {
            name: "notes".into(),
            path: "progens/notes".into(),
            url: None,
            managed: false,
            on_disk: true,
            is_git: false,
            head: None,
            pin_rev: None,
            pin_state: PinState::None,
            dirty: None,
            worktree_slots: None,
            worktree_orphans: None,
        }],
    };
    let human = format_status_human(&snap);
    assert!(!human.contains("worktrees"), "{human}");
    assert!(!human.contains("orphans"), "{human}");
}

#[test]
fn format_status_human_shows_orphans_when_non_empty() {
    let snap = StatusSnapshot {
        root: "/ws".into(),
        projects: vec![EntityStatus {
            name: "alpha".into(),
            path: "projects/alpha".into(),
            url: None,
            managed: false,
            on_disk: true,
            is_git: true,
            head: None,
            pin_rev: None,
            pin_state: PinState::None,
            dirty: Some(false),
            worktree_slots: Some(vec![WorktreeSlotInfo {
                name: "keep".into(),
                path: "worktrees/alpha/keep".into(),
                dirty: Some(false),
            }]),
            worktree_orphans: Some(vec![
                crate::worktree::WorktreeOrphanInfo {
                    name: "other".into(),
                    path: "worktrees/alpha/other".into(),
                },
                crate::worktree::WorktreeOrphanInfo {
                    name: "stale".into(),
                    path: "worktrees/alpha/stale".into(),
                },
            ]),
        }],
        progens: vec![],
    };
    let human = format_status_human(&snap);
    assert!(human.contains("worktrees: keep"), "{human}");
    assert!(human.contains("orphans: other, stale"), "{human}");
    // orphans line after worktrees
    let wt = human.find("worktrees:").expect("worktrees line");
    let or = human.find("orphans:").expect("orphans line");
    assert!(wt < or, "{human}");
}

#[test]
fn format_status_human_fully_empty_still_one_empty_message() {
    let snap = StatusSnapshot {
        root: "/ws".into(),
        projects: vec![],
        progens: vec![],
    };
    let human = format_status_human(&snap);
    assert_eq!(human, "Workspace: /ws\n(no projects or progens)\n");
}
