//! `odm-actions` — resolve cwd + shell-out Action dispatch.

use std::path::{Path, PathBuf};
use std::process::Command;

use odm_core::{
    abs_checkout, resolve_under_root, worktree_slot_path, ActionTask, OdmError, PathResolveError,
    Workspace,
};

/// How task stdio is handled during Action execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdioMode {
    /// Stream task stdout/stderr to the terminal (human `odm run`).
    Inherit,
    /// Capture per-task streams into [`TaskResult`] (machine `odm run --json`).
    Capture,
}

/// Where Action tasks run when no task `dir` is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CwdTarget<'a> {
    Root,
    Project { name: &'a str },
    Worktree { project: &'a str, slot: &'a str },
}

impl<'a> CwdTarget<'a> {
    /// Build a cwd target from CLI `--project` / `--wt` flags.
    pub fn from_flags(project: Option<&'a str>, wt: Option<&'a str>) -> Result<Self, OdmError> {
        match (project, wt) {
            (None, Some(_)) => Err(OdmError::usage("--wt requires --project")),
            (Some(p), Some(s)) => Ok(CwdTarget::Worktree {
                project: p,
                slot: s,
            }),
            (Some(p), None) => Ok(CwdTarget::Project { name: p }),
            (None, None) => Ok(CwdTarget::Root),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RunOptions<'a> {
    pub cwd: CwdTarget<'a>,
    pub extra_args: &'a [String],
    pub stdio: StdioMode,
}

/// One task's outcome. Streams are `Some` only under [`StdioMode::Capture`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskResult {
    pub exit_code: i32,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

/// Full Action run outcome (stop-on-first-failure; may be a prefix of tasks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    pub exit_code: i32,
    pub tasks: Vec<TaskResult>,
}

pub fn list_actions(ws: &Workspace) -> Vec<(&str, &odm_core::ActionDef)> {
    ws.actions
        .iter()
        .map(|(n, d)| (n.as_str(), d))
        .collect()
}

/// Resolve task cwd: task dir > worktree slot > project primary > workspace root.
pub fn resolve_cwd(
    ws: &Workspace,
    target: CwdTarget<'_>,
    task_dir: Option<&str>,
) -> Result<PathBuf, OdmError> {
    if let Some(dir) = task_dir {
        let cwd = resolve_under_root(ws.root.as_path(), dir).map_err(|e| match e {
            PathResolveError::Absolute { path } => {
                OdmError::usage(format!("action task dir must be relative, got '{path}'"))
            }
            PathResolveError::Escape { path } => OdmError::usage(format!(
                "action task dir must not escape Workspace root, got '{path}'"
            )),
        })?;
        if !cwd.is_dir() {
            return Err(OdmError::usage(format!(
                "action task dir does not exist: {dir}"
            )));
        }
        return Ok(cwd);
    }
    match target {
        CwdTarget::Root => Ok(ws.root.clone()),
        CwdTarget::Project { name } => {
            let entry = ws.config.projects.get(name).ok_or_else(|| {
                OdmError::usage(format!("unknown project '{name}'"))
            })?;
            let cwd = abs_checkout(ws.root.as_path(), &entry.path)?;
            if !cwd.is_dir() {
                return Err(OdmError::not_found(format!(
                    "project path missing: {}",
                    entry.path
                )));
            }
            Ok(cwd)
        }
        CwdTarget::Worktree { project, slot } => {
            if !ws.config.projects.contains_key(project) {
                return Err(OdmError::usage(format!("unknown project '{project}'")));
            }
            let cwd = worktree_slot_path(ws.root.as_path(), project, slot);
            if !cwd.is_dir() {
                return Err(OdmError::not_found(format!(
                    "worktree slot not found: worktrees/{project}/{slot}"
                )));
            }
            Ok(cwd)
        }
    }
}

pub fn run_action(
    ws: &Workspace,
    action_name: &str,
    opts: RunOptions<'_>,
) -> Result<RunResult, OdmError> {
    let def = ws.actions.get(action_name).ok_or_else(|| {
        OdmError::usage(format!("unknown action '{action_name}'"))
    })?;

    let mut tasks = Vec::new();
    let n = def.tasks.len();
    for (i, task) in def.tasks.iter().enumerate() {
        let cwd = resolve_cwd(ws, opts.cwd, task.dir.as_deref())?;
        let is_last = i + 1 == n;
        let extra = if is_last { opts.extra_args } else { &[] };
        let tr = run_task(task, &cwd, extra, opts.stdio)?;
        let code = tr.exit_code;
        tasks.push(tr);
        if code != 0 {
            return Ok(RunResult {
                exit_code: code,
                tasks,
            });
        }
    }
    Ok(RunResult {
        exit_code: 0,
        tasks,
    })
}

fn run_task(
    task: &ActionTask,
    cwd: &Path,
    extra_args: &[String],
    stdio: StdioMode,
) -> Result<TaskResult, OdmError> {
    let mut cmd = Command::new("sh");
    cmd.current_dir(cwd);
    if extra_args.is_empty() {
        cmd.arg("-c").arg(&task.run);
    } else {
        let script = format!("{} \"$@\"", task.run);
        cmd.arg("-c").arg(script).arg("_");
        for a in extra_args {
            cmd.arg(a);
        }
    }
    match stdio {
        StdioMode::Inherit => {
            let status = cmd.status().map_err(|e| {
                OdmError::operation(format!("failed to spawn shell for action: {e}"))
            })?;
            Ok(TaskResult {
                exit_code: status.code().unwrap_or(1),
                stdout: None,
                stderr: None,
            })
        }
        StdioMode::Capture => {
            let output = cmd.output().map_err(|e| {
                OdmError::operation(format!("failed to spawn shell for action: {e}"))
            })?;
            Ok(TaskResult {
                exit_code: output.status.code().unwrap_or(1),
                stdout: Some(String::from_utf8_lossy(&output.stdout).into_owned()),
                stderr: Some(String::from_utf8_lossy(&output.stderr).into_owned()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;

    use odm_core::{ActionDef, ActionTask, ProjectEntry, Workspace, WorkspaceConfig};
    use tempfile::tempdir;

    fn ws_with_actions(root: PathBuf, actions: BTreeMap<String, ActionDef>) -> Workspace {
        let mut config = WorkspaceConfig::default();
        config.projects.insert(
            "alpha".into(),
            ProjectEntry {
                path: "projects/alpha".into(),
                url: None,
                branch: None,
                type_: None,
                ..Default::default()
            },
        );
        Workspace {
            root,
            config,
            actions,
            generators: BTreeMap::new(),
        }
    }

    fn single(run: &str, dir: Option<&str>) -> ActionDef {
        ActionDef {
            tasks: vec![ActionTask {
                run: run.into(),
                dir: dir.map(str::to_string),
            }],
        }
    }

    fn inherit_root(extra: &[String]) -> RunOptions<'_> {
        RunOptions {
            cwd: CwdTarget::Root,
            extra_args: extra,
            stdio: StdioMode::Inherit,
        }
    }

    #[test]
    fn capture_collects_stdout() {
        let dir = tempdir().unwrap();
        let mut actions = BTreeMap::new();
        actions.insert("hello".into(), single("echo hello-desk", None));
        let ws = ws_with_actions(dir.path().to_path_buf(), actions);
        let result = run_action(
            &ws,
            "hello",
            RunOptions {
                cwd: CwdTarget::Root,
                extra_args: &[],
                stdio: StdioMode::Capture,
            },
        )
        .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.tasks.len(), 1);
        assert_eq!(result.tasks[0].exit_code, 0);
        let stdout = result.tasks[0].stdout.as_deref().unwrap();
        assert!(stdout.contains("hello-desk"), "got: {stdout}");
        assert!(result.tasks[0].stderr.is_some());
    }

    #[test]
    fn inherit_has_no_captured_streams() {
        let dir = tempdir().unwrap();
        let mut actions = BTreeMap::new();
        actions.insert("hello".into(), single("true", None));
        let ws = ws_with_actions(dir.path().to_path_buf(), actions);
        let result = run_action(&ws, "hello", inherit_root(&[])).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.tasks[0].stdout.is_none());
        assert!(result.tasks[0].stderr.is_none());
    }

    #[test]
    fn cwd_target_from_flags() {
        assert_eq!(CwdTarget::from_flags(None, None).unwrap(), CwdTarget::Root);
        assert_eq!(
            CwdTarget::from_flags(Some("alpha"), None).unwrap(),
            CwdTarget::Project { name: "alpha" }
        );
        assert_eq!(
            CwdTarget::from_flags(Some("alpha"), Some("slot1")).unwrap(),
            CwdTarget::Worktree {
                project: "alpha",
                slot: "slot1"
            }
        );
        let err = CwdTarget::from_flags(None, Some("slot1")).unwrap_err();
        assert!(err.to_string().contains("--wt requires --project"));
    }

    #[test]
    fn echo_success() {
        let dir = tempdir().unwrap();
        let mut actions = BTreeMap::new();
        actions.insert("hello".into(), single("echo hello-desk", None));
        let ws = ws_with_actions(dir.path().to_path_buf(), actions);
        let result = run_action(&ws, "hello", inherit_root(&[])).unwrap();
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn fail_exit_code() {
        let dir = tempdir().unwrap();
        let mut actions = BTreeMap::new();
        actions.insert("fail".into(), single("exit 7", None));
        let ws = ws_with_actions(dir.path().to_path_buf(), actions);
        let result = run_action(&ws, "fail", inherit_root(&[])).unwrap();
        assert_eq!(result.exit_code, 7);
    }

    #[test]
    fn multi_task_stop_on_fail() {
        let dir = tempdir().unwrap();
        let marker = dir.path().join("should-not-exist");
        let mut actions = BTreeMap::new();
        actions.insert(
            "chain".into(),
            ActionDef {
                tasks: vec![
                    ActionTask {
                        run: "exit 3".into(),
                        dir: None,
                    },
                    ActionTask {
                        run: format!("touch {}", marker.display()),
                        dir: None,
                    },
                ],
            },
        );
        let ws = ws_with_actions(dir.path().to_path_buf(), actions);
        let result = run_action(&ws, "chain", inherit_root(&[])).unwrap();
        assert_eq!(result.exit_code, 3);
        assert_eq!(result.tasks.len(), 1);
        assert!(!marker.exists());
    }

    #[test]
    fn cwd_dir() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir_all(&sub).unwrap();
        let out = dir.path().join("out.txt");
        let mut actions = BTreeMap::new();
        actions.insert(
            "pwd".into(),
            single(&format!("pwd > {}", out.display()), Some("sub")),
        );
        let ws = ws_with_actions(dir.path().to_path_buf(), actions);
        let result = run_action(&ws, "pwd", inherit_root(&[])).unwrap();
        assert_eq!(result.exit_code, 0);
        let text = fs::read_to_string(&out).unwrap();
        assert!(text.contains("sub"), "got: {text}");
    }

    #[test]
    fn extra_args_on_last_task() {
        let dir = tempdir().unwrap();
        let out = dir.path().join("args.txt");
        let mut actions = BTreeMap::new();
        actions.insert(
            "args".into(),
            ActionDef {
                tasks: vec![
                    ActionTask {
                        run: "true".into(),
                        dir: None,
                    },
                    ActionTask {
                        run: format!("printf '%s\\n' > {}", out.display()),
                        dir: None,
                    },
                ],
            },
        );
        let ws = ws_with_actions(dir.path().to_path_buf(), actions);
        let extras = vec!["one".into(), "two".into()];
        let result = run_action(&ws, "args", inherit_root(&extras)).unwrap();
        assert_eq!(result.exit_code, 0);
        let text = fs::read_to_string(&out).unwrap();
        assert_eq!(text, "one\ntwo\n");
    }

    #[test]
    fn unknown_action() {
        let dir = tempdir().unwrap();
        let ws = ws_with_actions(dir.path().to_path_buf(), BTreeMap::new());
        let err = run_action(&ws, "nope", inherit_root(&[])).unwrap_err();
        assert!(err.to_string().contains("unknown action"));
    }

    #[test]
    fn missing_task_dir() {
        let dir = tempdir().unwrap();
        let mut actions = BTreeMap::new();
        actions.insert("x".into(), single("true", Some("missing")));
        let ws = ws_with_actions(dir.path().to_path_buf(), actions);
        let err = run_action(&ws, "x", inherit_root(&[])).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn resolve_cwd_rejects_escape_dir() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.parent().unwrap().join("outside")).unwrap();
        let ws = ws_with_actions(root.to_path_buf(), BTreeMap::new());
        let err = resolve_cwd(&ws, CwdTarget::Root, Some("../outside")).unwrap_err();
        assert!(err.to_string().contains("escape"), "{err}");
    }

    #[test]
    fn resolve_cwd_rejects_absolute_dir() {
        let dir = tempdir().unwrap();
        let ws = ws_with_actions(dir.path().to_path_buf(), BTreeMap::new());
        let err = resolve_cwd(&ws, CwdTarget::Root, Some("/tmp")).unwrap_err();
        assert!(err.to_string().contains("relative"), "{err}");
    }

    #[test]
    fn resolve_cwd_in_workspace_dir() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("projects/alpha")).unwrap();
        let ws = ws_with_actions(root.to_path_buf(), BTreeMap::new());
        let cwd = resolve_cwd(&ws, CwdTarget::Root, Some("projects/alpha")).unwrap();
        assert_eq!(cwd, root.join("projects/alpha"));
    }

    #[test]
    fn resolve_cwd_priority() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("projects/alpha")).unwrap();
        fs::create_dir_all(root.join("worktrees/alpha/slot1")).unwrap();
        fs::create_dir_all(root.join("taskdir")).unwrap();
        let ws = ws_with_actions(root.to_path_buf(), BTreeMap::new());

        let task = resolve_cwd(
            &ws,
            CwdTarget::Worktree {
                project: "alpha",
                slot: "slot1",
            },
            Some("taskdir"),
        )
        .unwrap();
        assert_eq!(task, root.join("taskdir"));

        let wt = resolve_cwd(
            &ws,
            CwdTarget::Worktree {
                project: "alpha",
                slot: "slot1",
            },
            None,
        )
        .unwrap();
        assert_eq!(wt, root.join("worktrees/alpha/slot1"));

        let proj = resolve_cwd(&ws, CwdTarget::Project { name: "alpha" }, None).unwrap();
        assert_eq!(proj, root.join("projects/alpha"));

        let base = resolve_cwd(&ws, CwdTarget::Root, None).unwrap();
        assert_eq!(base, root);
    }

    #[test]
    fn run_action_project_cwd() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("projects/alpha")).unwrap();
        fs::write(root.join("projects/alpha/marker"), "proj\n").unwrap();
        let out = root.join("out.txt");
        let mut actions = BTreeMap::new();
        actions.insert(
            "cat".into(),
            single(&format!("cat marker > {}", out.display()), None),
        );
        let ws = ws_with_actions(root.to_path_buf(), actions);
        let result = run_action(
            &ws,
            "cat",
            RunOptions {
                cwd: CwdTarget::Project { name: "alpha" },
                extra_args: &[],
                stdio: StdioMode::Inherit,
            },
        )
        .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(fs::read_to_string(&out).unwrap(), "proj\n");
    }

    #[test]
    fn run_action_wt_cwd() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("projects/alpha")).unwrap();
        fs::create_dir_all(root.join("worktrees/alpha/slot1")).unwrap();
        fs::write(root.join("worktrees/alpha/slot1/marker"), "wt\n").unwrap();
        let out = root.join("out.txt");
        let mut actions = BTreeMap::new();
        actions.insert(
            "cat".into(),
            single(&format!("cat marker > {}", out.display()), None),
        );
        let ws = ws_with_actions(root.to_path_buf(), actions);
        let result = run_action(
            &ws,
            "cat",
            RunOptions {
                cwd: CwdTarget::Worktree {
                    project: "alpha",
                    slot: "slot1",
                },
                extra_args: &[],
                stdio: StdioMode::Inherit,
            },
        )
        .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(fs::read_to_string(&out).unwrap(), "wt\n");
    }

    #[test]
    fn run_action_wt_requires_project() {
        let err = CwdTarget::from_flags(None, Some("slot1")).unwrap_err();
        assert!(err.to_string().contains("--wt requires --project"));
    }

    #[test]
    fn resolve_cwd_missing_project_path_is_not_found() {
        let dir = tempdir().unwrap();
        let ws = ws_with_actions(dir.path().to_path_buf(), BTreeMap::new());
        let err = resolve_cwd(&ws, CwdTarget::Project { name: "alpha" }, None).unwrap_err();
        assert!(matches!(err, OdmError::NotFound(_)), "{err:?}");
        assert!(err.to_string().contains("project path missing"), "{err}");
    }

    #[test]
    fn resolve_cwd_unknown_project_is_usage() {
        let dir = tempdir().unwrap();
        let ws = ws_with_actions(dir.path().to_path_buf(), BTreeMap::new());
        let err = resolve_cwd(&ws, CwdTarget::Project { name: "nope" }, None).unwrap_err();
        assert!(matches!(err, OdmError::Usage(_)), "{err:?}");
        assert!(err.to_string().contains("unknown project"), "{err}");
    }

    #[test]
    fn resolve_cwd_missing_wt_slot_is_not_found() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("projects/alpha")).unwrap();
        let ws = ws_with_actions(root.to_path_buf(), BTreeMap::new());
        let err = resolve_cwd(
            &ws,
            CwdTarget::Worktree {
                project: "alpha",
                slot: "missing",
            },
            None,
        )
        .unwrap_err();
        assert!(matches!(err, OdmError::NotFound(_)), "{err:?}");
        assert!(err.to_string().contains("worktree slot not found"), "{err}");
    }

    #[test]
    fn resolve_cwd_unknown_project_on_wt_is_usage() {
        let dir = tempdir().unwrap();
        let ws = ws_with_actions(dir.path().to_path_buf(), BTreeMap::new());
        let err = resolve_cwd(
            &ws,
            CwdTarget::Worktree {
                project: "nope",
                slot: "slot1",
            },
            None,
        )
        .unwrap_err();
        assert!(matches!(err, OdmError::Usage(_)), "{err:?}");
        assert!(err.to_string().contains("unknown project"), "{err}");
    }
}
