use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use crate::error::{trim_output, GitError};
use crate::runner::{CommandRunner, ProcessRunner};

/// One row from `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
}

/// Index fact for one gitlink path. Porcelain prefixes never leave this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitlinkRecord {
    Missing,
    Recorded { sha: String },
    Conflict,
}

/// Shell-out git façade for ODM multi-git lifecycle.
///
/// Paths must be absolute. Library ops use `git -C <path>`, capture stdio, and
/// run non-interactively (`GIT_TERMINAL_PROMPT=0` via the process runner).
/// [`Git::run`] inherits stdio for `odm project git` passthrough and does not
/// force non-interactive env.
#[derive(Debug, Clone)]
pub struct Git<R: CommandRunner = ProcessRunner> {
    runner: R,
    program: OsString,
}

impl Default for Git<ProcessRunner> {
    fn default() -> Self {
        Self::new()
    }
}

impl Git<ProcessRunner> {
    pub fn new() -> Self {
        Self::with_runner(ProcessRunner)
    }
}

impl<R: CommandRunner> Git<R> {
    pub fn with_runner(runner: R) -> Self {
        Self {
            runner,
            program: OsString::from("git"),
        }
    }

    pub fn with_program(mut self, program: impl Into<OsString>) -> Self {
        self.program = program.into();
        self
    }

    pub fn is_repo(&self, path: &Path) -> Result<bool, GitError> {
        require_absolute(path)?;
        if !path.exists() {
            return Ok(false);
        }
        let out = self.capture(
            "is_repo",
            None,
            &["-C".into(), path.into(), "rev-parse".into(), "--is-inside-work-tree".into()],
        )?;
        if !out.status.success() {
            return Ok(false);
        }
        Ok(trim_output(out.stdout_str()) == "true")
    }

    /// True when `path` is the root of its own git worktree/repo (has a `.git`
    /// entry — directory or worktree file), not merely nested inside an ancestor.
    ///
    /// Used for entity observation (`is_git` / dirty): path-only vaults under a
    /// git Workspace must not inherit the parent repo's dirtiness.
    pub fn is_repo_root(&self, path: &Path) -> Result<bool, GitError> {
        require_absolute(path)?;
        if !path.join(".git").exists() {
            return Ok(false);
        }
        self.is_repo(path)
    }

    pub fn init(&self, path: &Path) -> Result<(), GitError> {
        require_absolute(path)?;
        let out = self.capture(
            "init",
            Some(path),
            &["init".into(), path.into()],
        )?;
        if !out.status.success() {
            return Err(GitError::failed(
                "init",
                Some(path.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        Ok(())
    }

    pub fn clone(
        &self,
        url: &str,
        path: &Path,
        branch: Option<&str>,
    ) -> Result<(), GitError> {
        require_absolute(path)?;
        let mut args: Vec<OsString> = vec!["clone".into()];
        if let Some(b) = branch {
            args.push("-b".into());
            args.push(b.into());
        }
        args.push(url.into());
        args.push(path.into());
        let out = self.capture("clone", Some(path), &args)?;
        if !out.status.success() {
            return Err(GitError::failed(
                "clone",
                Some(path.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        Ok(())
    }

    pub fn fetch(&self, path: &Path) -> Result<(), GitError> {
        require_absolute(path)?;
        let out = self.capture(
            "fetch",
            Some(path),
            &["-C".into(), path.into(), "fetch".into()],
        )?;
        if !out.status.success() {
            return Err(GitError::failed(
                "fetch",
                Some(path.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        Ok(())
    }

    pub fn head_sha(&self, path: &Path) -> Result<String, GitError> {
        require_absolute(path)?;
        let out = self.capture(
            "head_sha",
            Some(path),
            &["-C".into(), path.into(), "rev-parse".into(), "HEAD".into()],
        )?;
        if !out.status.success() {
            return Err(GitError::failed(
                "head_sha",
                Some(path.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        let sha = trim_output(out.stdout_str()).to_ascii_lowercase();
        if !is_full_sha(&sha) {
            return Err(GitError::Parse {
                operation: "head_sha",
                stdout: sha,
                detail: "expected 40-char hex SHA".into(),
            });
        }
        Ok(sha)
    }

    pub fn is_clean(&self, path: &Path) -> Result<bool, GitError> {
        require_absolute(path)?;
        let out = self.capture(
            "is_clean",
            Some(path),
            &[
                "-C".into(),
                path.into(),
                "status".into(),
                "--porcelain=v1".into(),
                "-uall".into(),
            ],
        )?;
        if !out.status.success() {
            return Err(GitError::failed(
                "is_clean",
                Some(path.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        Ok(out.stdout_str().trim().is_empty())
    }

    pub fn origin_url(&self, path: &Path) -> Result<String, GitError> {
        require_absolute(path)?;
        let out = self.capture(
            "origin_url",
            Some(path),
            &[
                "-C".into(),
                path.into(),
                "remote".into(),
                "get-url".into(),
                "origin".into(),
            ],
        )?;
        if !out.status.success() {
            let stderr = trim_output(out.stderr_str());
            if looks_like_missing_origin(&stderr) {
                return Err(GitError::OriginMissing {
                    path: path.to_path_buf(),
                });
            }
            return Err(GitError::failed(
                "origin_url",
                Some(path.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        Ok(trim_output(out.stdout_str()))
    }

    pub fn checkout_detached(&self, path: &Path, rev: &str) -> Result<(), GitError> {
        require_absolute(path)?;
        let out = self.capture(
            "checkout_detached",
            Some(path),
            &[
                "-C".into(),
                path.into(),
                "checkout".into(),
                "--detach".into(),
                rev.into(),
            ],
        )?;
        if !out.status.success() {
            return Err(GitError::failed(
                "checkout_detached",
                Some(path.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        Ok(())
    }

    /// Passthrough: `git -C <path> <args…>`. Inherits stdio. Empty args → [`GitError::EmptyArgs`].
    pub fn run(&self, path: &Path, args: &[impl AsRef<OsStr>]) -> Result<ExitStatus, GitError> {
        require_absolute(path)?;
        if args.is_empty() {
            return Err(GitError::EmptyArgs);
        }
        let mut full: Vec<OsString> = vec!["-C".into(), path.into()];
        full.extend(args.iter().map(|a| a.as_ref().to_os_string()));
        self.runner
            .status(&self.program, &full)
            .map_err(map_io)
    }

    /// `git -C <primary> worktree add [-b <branch>] -- <slot_path>`.
    ///
    /// Caller must ensure parent directories of `slot_path` exist.
    pub fn worktree_add(
        &self,
        primary: &Path,
        slot_path: &Path,
        branch: Option<&str>,
    ) -> Result<(), GitError> {
        require_absolute(primary)?;
        require_absolute(slot_path)?;
        let mut args: Vec<OsString> = vec![
            "-C".into(),
            primary.into(),
            "worktree".into(),
            "add".into(),
        ];
        if let Some(b) = branch {
            args.push("-b".into());
            args.push(b.into());
        }
        args.push("--".into());
        args.push(slot_path.into());
        let out = self.capture("worktree_add", Some(primary), &args)?;
        if !out.status.success() {
            return Err(GitError::failed(
                "worktree_add",
                Some(primary.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        Ok(())
    }

    /// `git -C <primary> worktree list --porcelain`.
    pub fn worktree_list(&self, primary: &Path) -> Result<Vec<WorktreeEntry>, GitError> {
        require_absolute(primary)?;
        let args: Vec<OsString> = vec![
            "-C".into(),
            primary.into(),
            "worktree".into(),
            "list".into(),
            "--porcelain".into(),
        ];
        let out = self.capture("worktree_list", Some(primary), &args)?;
        if !out.status.success() {
            return Err(GitError::failed(
                "worktree_list",
                Some(primary.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        parse_worktree_porcelain(&out.stdout_str())
    }

    /// `git -C <primary> worktree remove [--force] -- <slot_path>`.
    pub fn worktree_remove(
        &self,
        primary: &Path,
        slot_path: &Path,
        force: bool,
    ) -> Result<(), GitError> {
        require_absolute(primary)?;
        require_absolute(slot_path)?;
        let mut args: Vec<OsString> = vec![
            "-C".into(),
            primary.into(),
            "worktree".into(),
            "remove".into(),
        ];
        if force {
            args.push("--force".into());
        }
        args.push("--".into());
        args.push(slot_path.into());
        let out = self.capture("worktree_remove", Some(primary), &args)?;
        if !out.status.success() {
            return Err(GitError::failed(
                "worktree_remove",
                Some(primary.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        Ok(())
    }

    /// Index fact for `path` (repo-relative). Work tree occupancy is not a record.
    pub fn gitlink_record(&self, repo: &Path, path: &Path) -> Result<GitlinkRecord, GitError> {
        require_absolute(repo)?;
        let args: Vec<OsString> = vec![
            "-C".into(),
            repo.into(),
            "ls-files".into(),
            "--stage".into(),
            "--".into(),
            path.into(),
        ];
        let out = self.capture("gitlink_record", Some(repo), &args)?;
        if !out.status.success() {
            return Err(GitError::failed(
                "gitlink_record",
                Some(repo.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        let stdout = out.stdout_str();
        record_from_stage_rows(
            parse_ls_files_stage(&stdout, "gitlink_record")?,
            "gitlink_record",
            &stdout,
        )
    }

    /// Repo-relative paths with a `160000` index entry, including conflicts.
    pub fn list_gitlinks(&self, repo: &Path) -> Result<Vec<PathBuf>, GitError> {
        require_absolute(repo)?;
        let args: Vec<OsString> = vec![
            "-C".into(),
            repo.into(),
            "ls-files".into(),
            "--stage".into(),
        ];
        let out = self.capture("list_gitlinks", Some(repo), &args)?;
        if !out.status.success() {
            return Err(GitError::failed(
                "list_gitlinks",
                Some(repo.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        Ok(gitlink_paths(parse_ls_files_stage(
            &out.stdout_str(),
            "list_gitlinks",
        )?))
    }

    /// `git -C <repo> rm --cached -- <path>`. Does not commit. Leaves the work tree.
    pub fn unstage_gitlink(&self, repo: &Path, path: &Path) -> Result<(), GitError> {
        require_absolute(repo)?;
        let args: Vec<OsString> = vec![
            "-C".into(),
            repo.into(),
            "rm".into(),
            "--cached".into(),
            "--".into(),
            path.into(),
        ];
        let out = self.capture("unstage_gitlink", Some(repo), &args)?;
        if !out.status.success() {
            return Err(GitError::failed(
                "unstage_gitlink",
                Some(repo.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        Ok(())
    }

    /// `git -C <repo> submodule add [-b <branch>] -- <url> <path>`.
    /// Does not pass recurse or remote. Does not commit.
    pub fn submodule_add(
        &self,
        repo: &Path,
        url: &str,
        path: &Path,
        branch: Option<&str>,
    ) -> Result<(), GitError> {
        require_absolute(repo)?;
        let mut args: Vec<OsString> = vec![
            "-C".into(),
            repo.into(),
            "submodule".into(),
            "add".into(),
        ];
        if let Some(b) = branch {
            args.push("-b".into());
            args.push(b.into());
        }
        args.push("--".into());
        args.push(url.into());
        args.push(path.into());
        let out = self.capture("submodule_add", Some(repo), &args)?;
        if !out.status.success() {
            return Err(GitError::failed(
                "submodule_add",
                Some(repo.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        Ok(())
    }

    /// `git -C <repo> submodule update --init -- <path>`.
    /// Path-limited. Does not pass remote or recursive. Does not commit.
    pub fn submodule_update_init(&self, repo: &Path, path: &Path) -> Result<(), GitError> {
        require_absolute(repo)?;
        let args: Vec<OsString> = vec![
            "-C".into(),
            repo.into(),
            "submodule".into(),
            "update".into(),
            "--init".into(),
            "--".into(),
            path.into(),
        ];
        let out = self.capture("submodule_update_init", Some(repo), &args)?;
        if !out.status.success() {
            return Err(GitError::failed(
                "submodule_update_init",
                Some(repo.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        Ok(())
    }

    /// `git -C <repo> update-index --add --cacheinfo 160000,<rev>,<path>`.
    /// Does not commit. Does not use `git add`.
    pub fn update_gitlink(&self, repo: &Path, path: &Path, rev: &str) -> Result<(), GitError> {
        require_absolute(repo)?;
        let spec = format!("160000,{rev},{}", path.display());
        let args: Vec<OsString> = vec![
            "-C".into(),
            repo.into(),
            "update-index".into(),
            "--add".into(),
            "--cacheinfo".into(),
            spec.into(),
        ];
        let out = self.capture("update_gitlink", Some(repo), &args)?;
        if !out.status.success() {
            return Err(GitError::failed(
                "update_gitlink",
                Some(repo.to_path_buf()),
                out.status,
                out.stderr_str(),
                out.stdout_str(),
            ));
        }
        Ok(())
    }

    fn capture(
        &self,
        operation: &'static str,
        path: Option<&Path>,
        args: &[OsString],
    ) -> Result<crate::runner::CommandOutput, GitError> {
        self.runner
            .output(&self.program, args)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    GitError::GitNotFound(e)
                } else {
                    GitError::Failed {
                        operation,
                        path: path.map(Path::to_path_buf),
                        code: None,
                        stderr: e.to_string(),
                    }
                }
            })
    }
}

fn require_absolute(path: &Path) -> Result<(), GitError> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(GitError::NotAbsolute(path.to_path_buf()))
    }
}

fn map_io(e: std::io::Error) -> GitError {
    if e.kind() == std::io::ErrorKind::NotFound {
        GitError::GitNotFound(e)
    } else {
        GitError::Failed {
            operation: "run",
            path: None,
            code: None,
            stderr: e.to_string(),
        }
    }
}

fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn looks_like_missing_origin(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("no such remote") || lower.contains("not a remote")
}

struct StageRow {
    mode: String,
    sha: String,
    stage: u32,
    path: PathBuf,
}

fn parse_ls_files_stage(stdout: &str, operation: &'static str) -> Result<Vec<StageRow>, GitError> {
    stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| parse_ls_files_stage_line(line, operation, stdout))
        .collect()
}

fn parse_ls_files_stage_line(
    line: &str,
    operation: &'static str,
    raw: &str,
) -> Result<StageRow, GitError> {
    let parse_err = |detail: &str| GitError::Parse {
        operation,
        stdout: raw.to_string(),
        detail: detail.into(),
    };
    let (meta, path) = line
        .split_once('\t')
        .ok_or_else(|| parse_err("ls-files --stage line missing tab"))?;
    let mut bits = meta.split(' ');
    let mode = bits
        .next()
        .ok_or_else(|| parse_err("ls-files --stage line missing mode"))?;
    let sha = bits
        .next()
        .ok_or_else(|| parse_err("ls-files --stage line missing sha"))?;
    let stage = bits
        .next()
        .ok_or_else(|| parse_err("ls-files --stage line missing stage"))?;
    let stage: u32 = stage
        .parse()
        .map_err(|_| parse_err("ls-files --stage stage is not an integer"))?;
    Ok(StageRow {
        mode: mode.to_string(),
        sha: sha.to_ascii_lowercase(),
        stage,
        path: PathBuf::from(path),
    })
}

fn record_from_stage_rows(
    rows: Vec<StageRow>,
    operation: &'static str,
    stdout: &str,
) -> Result<GitlinkRecord, GitError> {
    if rows.is_empty() {
        return Ok(GitlinkRecord::Missing);
    }
    if rows.iter().any(|r| r.stage != 0) {
        return Ok(GitlinkRecord::Conflict);
    }
    match rows.as_slice() {
        [row] if row.mode == "160000" => {
            if !is_full_sha(&row.sha) {
                return Err(GitError::Parse {
                    operation,
                    stdout: stdout.to_string(),
                    detail: "expected 40-char hex SHA".into(),
                });
            }
            Ok(GitlinkRecord::Recorded {
                sha: row.sha.clone(),
            })
        }
        _ => Ok(GitlinkRecord::Missing),
    }
}

fn gitlink_paths(rows: Vec<StageRow>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for row in rows {
        if row.mode == "160000" && !paths.contains(&row.path) {
            paths.push(row.path);
        }
    }
    paths
}

/// Parse `git worktree list --porcelain` stdout into entries (including main).
fn parse_worktree_porcelain(stdout: &str) -> Result<Vec<WorktreeEntry>, GitError> {
    let mut entries = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut head: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut saw_line = false;

    let flush = |entries: &mut Vec<WorktreeEntry>,
                 path: &mut Option<PathBuf>,
                 head: &mut Option<String>,
                 branch: &mut Option<String>,
                 raw: &str|
     -> Result<(), GitError> {
        match path.take() {
            Some(p) => {
                entries.push(WorktreeEntry {
                    path: p,
                    head: head.take(),
                    branch: branch.take(),
                });
                Ok(())
            }
            None => Err(GitError::Parse {
                operation: "worktree_list",
                stdout: raw.to_string(),
                detail: "worktree record missing path".into(),
            }),
        }
    };

    for line in stdout.lines() {
        if line.is_empty() {
            if saw_line {
                flush(&mut entries, &mut path, &mut head, &mut branch, stdout)?;
                saw_line = false;
            }
            continue;
        }
        saw_line = true;
        if let Some(rest) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(rest));
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            head = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("branch ") {
            branch = Some(rest.to_string());
        }
        // bare / detached / locked / prunable — ignored
    }

    if saw_line || path.is_some() || head.is_some() || branch.is_some() {
        flush(&mut entries, &mut path, &mut head, &mut branch, stdout)?;
    }

    Ok(entries)
}

