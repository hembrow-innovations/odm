//! `odm-git` — multi-git lifecycle via shell-out to `git` on PATH (no libgit2).
//!
//! Public seam: [`Git`] typed ops. Policy (dirty/force, origin match) stays in core.

mod error;
mod git;
mod runner;

pub use error::GitError;
pub use git::{Git, GitlinkRecord, WorktreeEntry};
pub use runner::{CommandOutput, CommandRunner, ProcessRunner};
