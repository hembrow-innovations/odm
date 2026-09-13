use odm_git::Git;

use crate::config::Workspace;
use crate::doctor::{CheckStatus, DoctorCheck};
use crate::inventory::observe_project_worktrees;
use crate::paths::abs_checkout;

/// Orphan + dirty worktree checks from **one** inventory sample per project.
pub(crate) fn worktree_checks<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    for project in ws.config.projects.keys() {
        let Ok(inv) = observe_project_worktrees(git, ws, project) else {
            continue;
        };
        for orphan in inv.orphans {
            checks.push(DoctorCheck {
                id: format!("worktree_orphan:{project}:{}", orphan.name),
                status: CheckStatus::Warn,
                message: format!(
                    "orphan worktree slot directory (not a registered git worktree): {}",
                    orphan.path
                ),
                fixable: false,
            });
        }
        for slot in inv.slots {
            if slot.dirty != Some(true) {
                continue;
            }
            let rel = format!("worktrees/{project}/{}", slot.name);
            checks.push(DoctorCheck {
                id: format!("worktree_dirty:{project}:{}", slot.name),
                status: CheckStatus::Warn,
                message: format!("dirty worktree slot working tree: {rel}"),
                fixable: false,
            });
        }
    }
    checks
}

/// Warn on `worktrees/<project>/<slot>/` dirs that are not registered git worktrees.
/// Never fails; swallows git/path errors. Does not scan unknown project names under `worktrees/`.
///
/// Prefer [`worktree_checks`] in doctor so orphan+dirty share one sample.
#[allow(dead_code)]
pub(crate) fn worktree_orphan_checks<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    for (project, entry) in &ws.config.projects {
        let Ok(primary) = abs_checkout(&ws.root, &entry.path) else {
            continue;
        };
        if !primary.is_dir() {
            continue;
        }

        // No disk prefix → nothing to scan (skip git; same as pre-shared-helper).
        let project_wt = ws.root.join("worktrees").join(project);
        if !project_wt.is_dir() {
            continue;
        }

        let Ok(inv) = observe_project_worktrees(git, ws, project) else {
            continue;
        };

        for orphan in inv.orphans {
            checks.push(DoctorCheck {
                id: format!("worktree_orphan:{project}:{}", orphan.name),
                status: CheckStatus::Warn,
                message: format!(
                    "orphan worktree slot directory (not a registered git worktree): {}",
                    orphan.path
                ),
                fixable: false,
            });
        }
        let _ = inv.slots; // sample includes dirty probes; orphan path ignores them
    }
    checks
}

/// Warn on dirty registered worktree slots. Not fixable; soft-skips probe errors.
///
/// Prefer [`worktree_checks`] in doctor so orphan+dirty share one sample.
#[allow(dead_code)]
pub(crate) fn worktree_dirty_checks<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    for project in ws.config.projects.keys() {
        let Ok(inv) = observe_project_worktrees(git, ws, project) else {
            continue;
        };
        for slot in inv.slots {
            if slot.dirty != Some(true) {
                continue;
            }
            let rel = format!("worktrees/{project}/{}", slot.name);
            checks.push(DoctorCheck {
                id: format!("worktree_dirty:{project}:{}", slot.name),
                status: CheckStatus::Warn,
                message: format!("dirty worktree slot working tree: {rel}"),
                fixable: false,
            });
        }
    }
    checks
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::ExitStatus;
    use std::sync::{Arc, Mutex};

    use odm_git::{CommandOutput, CommandRunner};
    use tempfile::tempdir;

    use crate::config::{ProjectEntry, WorkspaceConfig};
    use crate::doctor::run_doctor;
    use crate::init::{init_workspace, InitOptions};
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

    fn ws_with_project(root: PathBuf, name: &str, rel: &str) -> Workspace {
        let mut projects = BTreeMap::new();
        projects.insert(
            name.into(),
            ProjectEntry {
                path: rel.into(),
                url: None,
                branch: None,
                type_: None,
                ..Default::default()
            },
        );
        Workspace {
            root,
            config: WorkspaceConfig {
                projects,
                manage_gitignore: Some(false),
                ..Default::default()
            },
            actions: BTreeMap::new(),
            generators: BTreeMap::new(),
        }
    }

    fn ensure_primary(root: &Path, rel: &str) -> PathBuf {
        let p = root.join(rel);
        fs::create_dir_all(&p).unwrap();
        // Own-repo marker so observation `is_repo_root` runs scripted is_repo.
        fs::write(p.join(".git"), "gitdir: mock\n").unwrap();
        p
    }

    #[test]
    fn orphan_slot_dir_warns_not_fixable() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let primary = ensure_primary(root, "projects/alpha");
        let orphan = worktree_slot_path(root, "alpha", "stale");
        fs::create_dir_all(&orphan).unwrap();
        let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
        let porcelain = format!(
            "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n",
            primary.display()
        );
        let (runner, _) = ScriptedRunner::new(vec![is_repo_true(), out_ok_stdout(&porcelain)]);
        let g = Git::with_runner(runner);
        let checks = worktree_orphan_checks(&g, &ws);
        assert_eq!(checks.len(), 1);
        let c = &checks[0];
        assert_eq!(c.id, "worktree_orphan:alpha:stale");
        assert_eq!(c.status, CheckStatus::Warn);
        assert!(!c.fixable);
        assert!(c.message.contains("worktrees/alpha/stale"));
    }

    #[test]
    fn registered_slot_is_not_orphan() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let primary = ensure_primary(root, "projects/alpha");
        let slot = worktree_slot_path(root, "alpha", "feat");
        fs::create_dir_all(&slot).unwrap();
        let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
        let porcelain = format!(
            "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n\
             worktree {}\nHEAD def\nbranch refs/heads/feat\n\n",
            primary.display(),
            slot.display(),
        );
        let (runner, _) = ScriptedRunner::new(vec![
            is_repo_true(),
            out_ok_stdout(&porcelain),
            out_ok_stdout(""),
        ]);
        let g = Git::with_runner(runner);
        let checks = worktree_orphan_checks(&g, &ws);
        assert!(
            checks.iter().all(|c| !c.id.contains("worktree_orphan")),
            "healthy slot must not warn: {checks:?}"
        );
    }

    #[test]
    fn missing_worktrees_dir_yields_no_orphan_checks() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        ensure_primary(root, "projects/alpha");
        let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
        let (runner, calls) = ScriptedRunner::new(vec![]);
        let g = Git::with_runner(runner);
        let checks = worktree_orphan_checks(&g, &ws);
        assert!(checks.is_empty());
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn non_git_primary_skips_orphan_scan() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        ensure_primary(root, "projects/alpha");
        let orphan = worktree_slot_path(root, "alpha", "stale");
        fs::create_dir_all(&orphan).unwrap();
        let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
        let (runner, _) = ScriptedRunner::new(vec![is_repo_false()]);
        let g = Git::with_runner(runner);
        let checks = worktree_orphan_checks(&g, &ws);
        assert!(checks.is_empty(), "non-git must not Fail/Warn orphans: {checks:?}");
    }

    #[test]
    fn unknown_worktrees_project_dir_ignored() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        ensure_primary(root, "projects/alpha");
        fs::create_dir_all(root.join("worktrees/ghost/slot")).unwrap();
        let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
        let (runner, calls) = ScriptedRunner::new(vec![]);
        let g = Git::with_runner(runner);
        let checks = worktree_orphan_checks(&g, &ws);
        assert!(checks.is_empty());
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn doctor_fix_does_not_delete_orphan_dirs() {
        let dir = tempdir().unwrap();
        init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: true,
            name: None,
        })
        .unwrap();
        let root = dir.path();
        let primary = ensure_primary(root, "projects/alpha");
        let orphan = worktree_slot_path(root, "alpha", "stale");
        fs::create_dir_all(&orphan).unwrap();
        let mut cfg = WorkspaceConfig {
            manage_gitignore: Some(false),
            ..Default::default()
        };
        cfg.projects.insert(
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
            config: cfg,
            actions: Default::default(),
            generators: Default::default(),
        };
        let porcelain = format!(
            "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n",
            primary.display()
        );
        let (runner, _) = ScriptedRunner::new(vec![
            is_repo_true(),
            out_ok_stdout("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n"),
            out_ok_stdout("https://example.com/alpha.git\n"),
            out_ok_stdout(""),
            is_repo_true(),
            out_ok_stdout(&porcelain),
        ]);
        let g = Git::with_runner(runner);
        let report = run_doctor(&g, &ws, true).unwrap();
        assert!(orphan.is_dir(), "doctor --fix must not delete orphan dirs");
        let c = report
            .checks
            .iter()
            .find(|c| c.id == "worktree_orphan:alpha:stale")
            .expect("orphan warn present");
        assert_eq!(c.status, CheckStatus::Warn);
        assert!(!c.fixable);
        assert!(report.ok);
        assert!(
            report
                .checks
                .iter()
                .all(|c| !c.id.starts_with("worktree_dirty:")),
            "orphan must not produce dirty checks: {:?}",
            report.checks
        );
    }

    #[test]
    fn invalid_slot_name_dir_skipped_silently() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let primary = ensure_primary(root, "projects/alpha");
        let project_wt = root.join("worktrees/alpha");
        fs::create_dir_all(&project_wt).unwrap();
        fs::write(project_wt.join("not-a-dir"), b"x").unwrap();
        let orphan = worktree_slot_path(root, "alpha", "real");
        fs::create_dir_all(&orphan).unwrap();
        let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
        let porcelain = format!(
            "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n",
            primary.display()
        );
        let (runner, _) = ScriptedRunner::new(vec![is_repo_true(), out_ok_stdout(&porcelain)]);
        let g = Git::with_runner(runner);
        let checks = worktree_orphan_checks(&g, &ws);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].id, "worktree_orphan:alpha:real");
    }

    #[test]
    fn dirty_registered_slot_warns_not_fixable() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let primary = ensure_primary(root, "projects/alpha");
        let slot = worktree_slot_path(root, "alpha", "feat");
        fs::create_dir_all(&slot).unwrap();
        let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
        let porcelain = format!(
            "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n\
             worktree {}\nHEAD def\nbranch refs/heads/feat\n\n",
            primary.display(),
            slot.display(),
        );
        let (runner, _) = ScriptedRunner::new(vec![
            is_repo_true(),
            out_ok_stdout(&porcelain),
            out_ok_stdout(" M dirty.txt\n"),
        ]);
        let g = Git::with_runner(runner);
        let checks = worktree_dirty_checks(&g, &ws);
        assert_eq!(checks.len(), 1);
        let c = &checks[0];
        assert_eq!(c.id, "worktree_dirty:alpha:feat");
        assert_eq!(c.status, CheckStatus::Warn);
        assert!(!c.fixable);
        assert!(c.message.contains("worktrees/alpha/feat"));
    }

    #[test]
    fn clean_registered_slot_no_dirty_check() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let primary = ensure_primary(root, "projects/alpha");
        let slot = worktree_slot_path(root, "alpha", "feat");
        fs::create_dir_all(&slot).unwrap();
        let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
        let porcelain = format!(
            "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n\
             worktree {}\nHEAD def\nbranch refs/heads/feat\n\n",
            primary.display(),
            slot.display(),
        );
        let (runner, _) = ScriptedRunner::new(vec![
            is_repo_true(),
            out_ok_stdout(&porcelain),
            out_ok_stdout(""),
        ]);
        let g = Git::with_runner(runner);
        let checks = worktree_dirty_checks(&g, &ws);
        assert!(
            checks.iter().all(|c| !c.id.starts_with("worktree_dirty:")),
            "clean slot must not warn: {checks:?}"
        );
    }

    #[test]
    fn orphan_dir_is_not_dirty_checked() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let primary = ensure_primary(root, "projects/alpha");
        let orphan = worktree_slot_path(root, "alpha", "stale");
        fs::create_dir_all(&orphan).unwrap();
        let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
        let porcelain = format!(
            "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n",
            primary.display()
        );
        let (runner, _) = ScriptedRunner::new(vec![is_repo_true(), out_ok_stdout(&porcelain)]);
        let g = Git::with_runner(runner);
        let checks = worktree_checks(&g, &ws);
        let orphans: Vec<_> = checks
            .iter()
            .filter(|c| c.id.starts_with("worktree_orphan:"))
            .collect();
        let dirty: Vec<_> = checks
            .iter()
            .filter(|c| c.id.starts_with("worktree_dirty:"))
            .collect();
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].id, "worktree_orphan:alpha:stale");
        assert!(dirty.is_empty(), "orphan must not get dirty check: {dirty:?}");
    }

    #[test]
    fn is_clean_err_skips_slot() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let primary = ensure_primary(root, "projects/alpha");
        let slot = worktree_slot_path(root, "alpha", "feat");
        fs::create_dir_all(&slot).unwrap();
        let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
        let porcelain = format!(
            "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n\
             worktree {}\nHEAD def\nbranch refs/heads/feat\n\n",
            primary.display(),
            slot.display(),
        );
        let (runner, _) = ScriptedRunner::new(vec![
            is_repo_true(),
            out_ok_stdout(&porcelain),
            out_fail("status failed"),
        ]);
        let g = Git::with_runner(runner);
        let checks = worktree_dirty_checks(&g, &ws);
        assert!(checks.is_empty(), "probe error must skip: {checks:?}");
    }

    #[test]
    fn non_git_primary_skips_dirty_scan() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        ensure_primary(root, "projects/alpha");
        let slot = worktree_slot_path(root, "alpha", "feat");
        fs::create_dir_all(&slot).unwrap();
        let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
        let (runner, _) = ScriptedRunner::new(vec![is_repo_false()]);
        let g = Git::with_runner(runner);
        let checks = worktree_dirty_checks(&g, &ws);
        assert!(checks.is_empty(), "non-git must not dirty-check: {checks:?}");
    }

    #[test]
    fn doctor_fix_does_not_clean_dirty_slot() {
        let dir = tempdir().unwrap();
        init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: true,
            name: None,
        })
        .unwrap();
        let root = dir.path();
        let primary = ensure_primary(root, "projects/alpha");
        let slot = worktree_slot_path(root, "alpha", "feat");
        fs::create_dir_all(&slot).unwrap();
        let dirty_file = slot.join("dirty.txt");
        fs::write(&dirty_file, b"uncommitted").unwrap();
        let mut cfg = WorkspaceConfig {
            manage_gitignore: Some(false),
            ..Default::default()
        };
        cfg.projects.insert(
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
            config: cfg,
            actions: Default::default(),
            generators: Default::default(),
        };
        let porcelain = format!(
            "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n\
             worktree {}\nHEAD def\nbranch refs/heads/feat\n\n",
            primary.display(),
            slot.display(),
        );
        let (runner, _) = ScriptedRunner::new(vec![
            is_repo_true(),
            out_ok_stdout("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n"),
            out_ok_stdout("https://example.com/alpha.git\n"),
            out_ok_stdout(""),
            is_repo_true(),
            out_ok_stdout(&porcelain),
            out_ok_stdout("?? dirty.txt\n"),
        ]);
        let g = Git::with_runner(runner);
        let report = run_doctor(&g, &ws, true).unwrap();
        assert!(
            dirty_file.is_file(),
            "doctor --fix must not clean/stash slot trees"
        );
        let c = report
            .checks
            .iter()
            .find(|c| c.id == "worktree_dirty:alpha:feat")
            .expect("dirty warn present");
        assert_eq!(c.status, CheckStatus::Warn);
        assert!(!c.fixable);
        assert!(report.ok);
    }

    #[test]
    fn combined_checks_one_list_per_project() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let primary = ensure_primary(root, "projects/alpha");
        let slot = worktree_slot_path(root, "alpha", "feat");
        let orphan = worktree_slot_path(root, "alpha", "stale");
        fs::create_dir_all(&slot).unwrap();
        fs::create_dir_all(&orphan).unwrap();
        let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
        let porcelain = format!(
            "worktree {}\nHEAD abc\nbranch refs/heads/main\n\n\
             worktree {}\nHEAD def\nbranch refs/heads/feat\n\n",
            primary.display(),
            slot.display(),
        );
        let (runner, calls) = ScriptedRunner::new(vec![
            is_repo_true(),
            out_ok_stdout(&porcelain),
            out_ok_stdout(" M x\n"),
        ]);
        let g = Git::with_runner(runner);
        let checks = worktree_checks(&g, &ws);
        assert!(checks.iter().any(|c| c.id == "worktree_orphan:alpha:stale"));
        assert!(checks.iter().any(|c| c.id == "worktree_dirty:alpha:feat"));
        assert_eq!(calls.lock().unwrap().len(), 3);
    }
}
