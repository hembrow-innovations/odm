use std::process::ExitCode;

use clap::error::ErrorKind;
use clap::Parser;
use odm::cli::{
    resolve_wt_from_env, Cli, Commands, PinCmd, ProgenCmd, ProjectCmd, ProjectWorktreeCmd,
};
use odm::commands::{
    backlinks_cmd, body_cmd, context_cmd, doctor_cmd, find_cmd, finish_run, generate_cmd, get_cmd,
    init_cmd, ls_cmd, pin_apply_cmd, pin_record_cmd, pin_status_cmd, progen_add_cmd, progen_doctor_cmd,
    progen_info_cmd, progen_list_cmd, progen_rm_cmd, project_add_cmd, project_git_cmd,
    project_info_cmd, project_list_cmd, project_rm_cmd, reindex_cmd, run_cmd, status_cmd, sync_cmd,
    tree_cmd, worktree_add_cmd, worktree_list_cmd, worktree_prune_cmd, worktree_rm_cmd,
};
use odm::ctx::Ctx;
use odm::present::{finish, print_error, GlobalOut};
use odm_core::OdmError;

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => return exit_clap_error(e),
    };
    let out = GlobalOut { json: cli.json };

    match run(cli, &out) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            let code = print_error(&out, &e);
            ExitCode::from(code as u8)
        }
    }
}

fn exit_clap_error(e: clap::Error) -> ExitCode {
    match e.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            let _ = e.print();
            ExitCode::SUCCESS
        }
        _ => {
            let json = std::env::args_os().any(|a| a == "--json");
            if json {
                let msg = e.to_string().trim().to_string();
                print_error(&GlobalOut { json: true }, &OdmError::usage(msg));
            } else {
                let _ = e.print();
            }
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli, out: &GlobalOut) -> Result<i32, OdmError> {
    // Suppress unused clap help field (execution uses resolve_wt_from_env).
    let _ = &cli.wt;

    match cli.command {
        Commands::Init {
            path,
            no_git,
            interactive,
        } => finish(out, &init_cmd(path, no_git, interactive)?),

        cmd => {
            let wt = resolve_wt_from_env()?;
            let mut ctx = Ctx::open(
                cli.root.as_deref(),
                cli.project.clone(),
                wt,
                cli.progen.clone(),
                cli.progen_group.clone(),
            )?;
            dispatch(&mut ctx, out, cmd)
        }
    }
}

fn dispatch(ctx: &mut Ctx, out: &GlobalOut, cmd: Commands) -> Result<i32, OdmError> {
    match cmd {
        Commands::Init { .. } => unreachable!("init handled before context open"),
        Commands::Sync { names } => finish(out, &sync_cmd(ctx, &names)?),
        Commands::Pin { cmd } => match cmd {
            PinCmd::Record { names, force } => finish(out, &pin_record_cmd(ctx, &names, force)?),
            PinCmd::Apply { names, force } => finish(out, &pin_apply_cmd(ctx, &names, force)?),
            PinCmd::Status { names } => finish(out, &pin_status_cmd(ctx, &names)?),
        },
        Commands::Status => finish(out, &status_cmd(ctx)?),
        Commands::Doctor { fix } => finish(out, &doctor_cmd(ctx, fix)?),
        Commands::Project { cmd } => match cmd {
            ProjectCmd::List => finish(out, &project_list_cmd(ctx)?),
            ProjectCmd::Add {
                name,
                path,
                url,
                branch,
                type_,
                no_clone,
                gitlink,
            } => finish(
                out,
                &project_add_cmd(ctx, &name, &path, url, branch, type_, no_clone, gitlink)?,
            ),
            ProjectCmd::Rm {
                name,
                delete,
                force,
            } => finish(out, &project_rm_cmd(ctx, &name, delete, force)?),
            ProjectCmd::Info { name } => finish(out, &project_info_cmd(ctx, &name)?),
            ProjectCmd::Git { name, git_args } => {
                let status = project_git_cmd(ctx, &name, &git_args)?;
                Ok(status.code().unwrap_or(1))
            }
            ProjectCmd::Worktree { cmd } => match cmd {
                ProjectWorktreeCmd::List { project } => {
                    finish(out, &worktree_list_cmd(ctx, &project)?)
                }
                ProjectWorktreeCmd::Add {
                    project,
                    slot,
                    branch,
                } => finish(
                    out,
                    &worktree_add_cmd(ctx, &project, &slot, branch.as_deref())?,
                ),
                ProjectWorktreeCmd::Rm {
                    project,
                    slot,
                    force,
                } => finish(out, &worktree_rm_cmd(ctx, &project, &slot, force)?),
                ProjectWorktreeCmd::Prune {
                    project,
                    all,
                    force,
                } => finish(out, &worktree_prune_cmd(ctx, project, all, force)?),
            },
        },
        Commands::Progen { cmd } => match cmd {
            ProgenCmd::List => finish(out, &progen_list_cmd(ctx)?),
            ProgenCmd::Add {
                name,
                path,
                url,
                branch,
                no_clone,
                gitlink,
            } => finish(
                out,
                &progen_add_cmd(ctx, &name, &path, url, branch, no_clone, gitlink)?,
            ),
            ProgenCmd::Rm {
                name,
                delete,
                force,
            } => finish(out, &progen_rm_cmd(ctx, &name, delete, force)?),
            ProgenCmd::Info { name } => finish(out, &progen_info_cmd(ctx, &name)?),
            ProgenCmd::Get { id } => finish(out, &get_cmd(ctx, &id)?),
            ProgenCmd::Body { id } => finish(out, &body_cmd(ctx, &id)?),
            ProgenCmd::Tree => finish(out, &tree_cmd(ctx)?),
            ProgenCmd::Backlinks { id } => finish(out, &backlinks_cmd(ctx, &id)?),
            ProgenCmd::Ls => finish(out, &ls_cmd(ctx)?),
            ProgenCmd::Reindex => finish(out, &reindex_cmd(ctx)?),
            ProgenCmd::Doctor => finish(out, &progen_doctor_cmd(ctx)?),
        },
        Commands::Find { query, limit } => finish(out, &find_cmd(ctx, query, limit)?),
        Commands::Context { id } => finish(
            out,
            &context_cmd(
                ctx,
                &id,
                "context accepts at most one --progen (or use name:id)",
            )?,
        ),
        Commands::Run { action, extra } => finish_run(out, run_cmd(ctx, action, &extra, out.json)?),
        Commands::Generate {
            name,
            dest,
            force,
            dry_run,
        } => finish(out, &generate_cmd(ctx, name, dest, force, dry_run)?),
    }
}
