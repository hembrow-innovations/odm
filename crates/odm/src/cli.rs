use std::path::PathBuf;

use clap::{Parser, Subcommand};
use odm_core::OdmError;

#[derive(Debug, Parser)]
#[command(name = "odm", version, about = "Orchestrated Development Management")]
pub struct Cli {
    /// Workspace root (must contain `.odm/odm.config.yaml`). No upward walk.
    #[arg(long, global = true)]
    pub root: Option<PathBuf>,

    /// Machine-readable JSON on stdout.
    #[arg(long, global = true)]
    pub json: bool,

    /// Include/select Progen by config name (repeatable).
    #[arg(long = "progen", global = true)]
    pub progen: Vec<String>,

    /// Include members of a Progen group (repeatable).
    #[arg(long = "progen-group", global = true)]
    pub progen_group: Vec<String>,

    /// Project name for commands that target a project (e.g. run).
    #[arg(long, global = true)]
    pub project: Option<String>,

    /// Worktree slot name (with --project). Repeatable; conflicting values → usage error.
    /// Help surface only — execution resolves via [`resolve_wt_from_env`] (clap global Append
    /// drops values when `--wt` appears both before and after the subcommand).
    #[arg(long, global = true, action = clap::ArgAction::Append)]
    pub wt: Vec<String>,

    #[command(subcommand)]
    pub command: Commands,
}

/// Resolve `--wt` once from process argv (single source of truth for execution).
pub fn resolve_wt_from_env() -> Result<Option<String>, OdmError> {
    resolve_wt_flags(&collect_wt_from_argv(std::env::args()))
}

/// Collect every `--wt` / `--wt=` before `--` (clap global Append drops split positions).
pub fn collect_wt_from_argv(args: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut iter = args.into_iter();
    let _ = iter.next(); // skip argv0
    while let Some(a) = iter.next() {
        if a == "--" {
            break;
        }
        if a == "--wt" {
            if let Some(v) = iter.next() {
                if v != "--" && !v.starts_with('-') {
                    out.push(v);
                }
            }
        } else if let Some(rest) = a.strip_prefix("--wt=") {
            out.push(rest.to_string());
        }
    }
    out
}

/// Collapse repeated `--wt` flags: none / one / equal repeats OK; differing → usage.
pub fn resolve_wt_flags(flags: &[String]) -> Result<Option<String>, OdmError> {
    match flags {
        [] => Ok(None),
        [w] => Ok(Some(w.clone())),
        [first, rest @ ..] => {
            if rest.iter().all(|w| w == first) {
                Ok(Some(first.clone()))
            } else {
                Err(OdmError::usage(format!(
                    "conflicting --wt values: {}",
                    flags.join(", ")
                )))
            }
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Bootstrap a new Workspace.
    Init {
        /// Target path (default: cwd).
        path: Option<PathBuf>,

        /// Skip `git init` at Workspace root.
        #[arg(long)]
        no_git: bool,

        /// Interactive prompts (not implemented day one).
        #[arg(short = 'i', long)]
        interactive: bool,
    },

    /// Materialize managed entries and fetch (not checkout).
    Sync {
        /// Entity names (default: all managed).
        names: Vec<String>,
    },

    /// Pin file apply / status.
    Pin {
        #[command(subcommand)]
        cmd: PinCmd,
    },

    /// Workspace snapshot.
    Status,

    /// ODM-side health checks.
    Doctor {
        /// Mechanical repairs only.
        #[arg(long)]
        fix: bool,
    },

    /// Project lifecycle.
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },

    /// Progen lifecycle + store façade.
    Progen {
        #[command(subcommand)]
        cmd: ProgenCmd,
    },

    /// Federated vault search (FTS).
    Find {
        /// Free-text query (empty = list scoped notes).
        query: Option<String>,
        /// Max hits per Progen store (default 200).
        #[arg(long, default_value_t = 200)]
        limit: usize,
    },

    /// In-store note neighborhood via wikilinks.
    Context {
        /// Note id, or `progen:id`.
        id: String,
    },

    /// List or run an Action from merged bundles.
    Run {
        /// Action name (omit to list).
        action: Option<String>,
        /// Extra args after `--`
        #[arg(last = true)]
        extra: Vec<String>,
    },

    /// List generators or materialize a local template.
    Generate {
        /// Generator name (omit to list).
        name: Option<String>,
        /// Destination path relative to Workspace root (required with name).
        #[arg(long, requires = "name")]
        dest: Option<PathBuf>,
        /// Overwrite files when destination is non-empty.
        #[arg(long, requires = "name")]
        force: bool,
        /// Preview without writing files.
        #[arg(long, requires = "name")]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum PinCmd {
    /// Checkout pinned revs as detached HEAD.
    Apply {
        names: Vec<String>,
        #[arg(long)]
        force: bool,
    },
    /// Compare pin file vs current HEAD.
    Status { names: Vec<String> },
}

#[derive(Debug, Subcommand)]
pub enum ProjectCmd {
    /// List configured projects.
    List,
    /// Add a project entry.
    Add {
        name: String,
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long = "type")]
        type_: Option<String>,
        #[arg(long)]
        no_clone: bool,
        #[arg(long, requires = "url")]
        gitlink: bool,
    },
    /// Remove a project entry.
    Rm {
        name: String,
        #[arg(long)]
        delete: bool,
        #[arg(long)]
        force: bool,
    },
    /// Show one project.
    Info { name: String },
    /// Run git in a project checkout.
    Git {
        name: String,
        #[arg(last = true)]
        git_args: Vec<String>,
    },
    /// Worktree slot lifecycle.
    Worktree {
        #[command(subcommand)]
        cmd: ProjectWorktreeCmd,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProjectWorktreeCmd {
    /// List worktree slots for a project.
    List {
        /// Project name.
        project: String,
    },
    /// Add a worktree slot.
    Add {
        /// Project name.
        project: String,
        /// Slot name (simple path segment).
        slot: String,
        /// Create and check out a new branch.
        #[arg(long)]
        branch: Option<String>,
    },
    /// Remove a worktree slot.
    Rm {
        /// Project name.
        project: String,
        /// Slot name.
        slot: String,
        /// Force remove (dirty worktree).
        #[arg(long)]
        force: bool,
    },
    /// Remove orphan slot directories under worktrees/<project>/.
    Prune {
        /// Project name (required unless `--all`).
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        project: Option<String>,
        /// Prune orphans for every configured project.
        #[arg(long)]
        all: bool,
        /// Recursively delete non-empty orphan dirs.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProgenCmd {
    /// List configured progens.
    List,
    /// Add a progen (Obsidian-compatible vault path).
    Add {
        name: String,
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long)]
        no_clone: bool,
        #[arg(long, requires = "url")]
        gitlink: bool,
    },
    /// Remove a progen entry.
    Rm {
        name: String,
        #[arg(long)]
        delete: bool,
        #[arg(long)]
        force: bool,
    },
    /// Show one progen / vault info.
    Info { name: String },
    /// Get a note by id (single-root).
    Get { id: String },
    /// Print note body only (single-root).
    Body { id: String },
    /// List note paths as a sorted tree (single-root).
    Tree,
    /// Notes that wikilink to id (single-root).
    Backlinks { id: String },
    /// List notes in a progen (single-root).
    Ls,
    /// Rebuild disposable index under `.odm/progen/<name>/`.
    Reindex,
    /// Store-side health (path + index).
    Doctor,
}

#[cfg(test)]
mod wt_flag_tests {
    use super::{collect_wt_from_argv, resolve_wt_flags};

    #[test]
    fn collect_split_and_equals() {
        let args = vec![
            "odm".into(),
            "--wt".into(),
            "a".into(),
            "project".into(),
            "git".into(),
            "p".into(),
            "--wt=b".into(),
            "--".into(),
            "status".into(),
        ];
        assert_eq!(collect_wt_from_argv(args), vec!["a", "b"]);
    }

    #[test]
    fn resolve_conflict_and_equal() {
        assert!(resolve_wt_flags(&[]).unwrap().is_none());
        assert_eq!(
            resolve_wt_flags(&["x".into()]).unwrap().as_deref(),
            Some("x")
        );
        assert_eq!(
            resolve_wt_flags(&["x".into(), "x".into()])
                .unwrap()
                .as_deref(),
            Some("x")
        );
        assert!(resolve_wt_flags(&["a".into(), "b".into()]).is_err());
    }
}
