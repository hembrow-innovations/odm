//! Workspace inventory sample — worktree slots/orphans.

use std::collections::BTreeSet;

use odm_git::Git;

use crate::config::Workspace;
use crate::error::OdmError;
use crate::worktree::{
    worktree_list, worktree_registered_names, worktree_orphan_infos, WorktreeOrphanInfo,
    WorktreeSlotInfo,
};

/// Per-project worktree inventory (registered slots with dirty + orphans).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorktreeInventory {
    pub project: String,
    pub slots: Vec<WorktreeSlotInfo>,
    pub orphans: Vec<WorktreeOrphanInfo>,
}

/// Sample worktrees for one project: slots (with dirty) + orphans.
/// Propagates list/primary errors; callers soft-fail as needed.
pub fn observe_project_worktrees<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
    project: &str,
) -> Result<ProjectWorktreeInventory, OdmError> {
    let list = worktree_list(git, ws, project)?;
    let registered: BTreeSet<String> = list.slots.iter().map(|s| s.name.clone()).collect();
    let orphans = worktree_orphan_infos(ws, project, &registered);
    Ok(ProjectWorktreeInventory {
        project: list.project,
        slots: list.slots,
        orphans,
    })
}

/// Soft-fail sample: list/git errors → empty slots and orphans.
pub fn observe_project_worktrees_soft<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
    project: &str,
) -> ProjectWorktreeInventory {
    match observe_project_worktrees(git, ws, project) {
        Ok(inv) => inv,
        Err(_) => ProjectWorktreeInventory {
            project: project.to_string(),
            slots: vec![],
            orphans: vec![],
        },
    }
}

/// Registered slot names without dirty probes (prune name-set path).
pub fn observe_worktree_registered_names<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
    project: &str,
) -> Result<BTreeSet<String>, OdmError> {
    worktree_registered_names(git, ws, project)
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
        p
    }

    #[test]
    fn observe_worktrees_slots_and_orphans_one_list() {
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
            out_ok_stdout(""), // is_clean feat
        ]);
        let g = Git::with_runner(runner);
        let inv = observe_project_worktrees(&g, &ws, "alpha").unwrap();
        assert_eq!(inv.slots.len(), 1);
        assert_eq!(inv.slots[0].name, "feat");
        assert_eq!(inv.slots[0].dirty, Some(false));
        assert_eq!(inv.orphans.len(), 1);
        assert_eq!(inv.orphans[0].name, "stale");
        // is_repo + worktree list + one is_clean — not two lists
        let n = calls.lock().unwrap().len();
        assert_eq!(n, 3, "expected one list sample, got {n} calls");
    }

    #[test]
    fn observe_worktrees_soft_fail_empty() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        ensure_primary(root, "projects/alpha");
        let ws = ws_with_project(root.to_path_buf(), "alpha", "projects/alpha");
        let (runner, _) = ScriptedRunner::new(vec![out_fail("not a repo")]);
        let g = Git::with_runner(runner);
        let inv = observe_project_worktrees_soft(&g, &ws, "alpha");
        assert!(inv.slots.is_empty());
        assert!(inv.orphans.is_empty());
        assert_eq!(inv.project, "alpha");
    }

    #[test]
    fn registered_names_skips_dirty_probes() {
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
        let (runner, calls) =
            ScriptedRunner::new(vec![is_repo_true(), out_ok_stdout(&porcelain)]);
        let g = Git::with_runner(runner);
        let names = observe_worktree_registered_names(&g, &ws, "alpha").unwrap();
        assert!(names.contains("feat"));
        let args = calls.lock().unwrap();
        assert_eq!(args.len(), 2, "no is_clean: {:?}", *args);
        assert!(
            args.iter().all(|a| {
                !a.iter().any(|x| x == "status")
            }),
            "must not dirty-probe: {:?}",
            *args
        );
    }
}
