//! Library command handlers returning presentable DTOs.

mod find;
mod generate;
mod materialize;
mod project;
mod progen;
mod run;
mod worktree;
mod workspace;

pub use find::{find_cmd, find_notes_dto, FindDto};
pub use generate::{
    format_generate_run_human, format_generator_list_human, generate_cmd, list_generators_dto,
    GenerateRunDto, GeneratorListDto, GeneratorListItem,
};
pub use materialize::{
    format_progen_add_human, format_project_add_human, materialize_json, materialize_json_opt,
    materialize_sync_human, MaterializeLabel,
};
pub use project::{
    add_cmd as project_add_cmd, format_project_info_human, format_project_list_human,
    git_cmd as project_git_cmd, info_cmd as project_info_cmd, list_cmd as project_list_cmd,
    list_projects, project_info, project_info_from, project_list_from, rm_cmd as project_rm_cmd,
    ProjectInfoDto, ProjectListDto, ProjectListItem,
};
pub use progen::{
    add_cmd as progen_add_cmd, backlinks_cmd, body_cmd, context_cmd, doctor_cmd as progen_doctor_cmd,
    format_progen_info_human, format_progen_list_human, get_cmd, info_cmd as progen_info_cmd,
    list_cmd as progen_list_cmd, list_progens, ls_cmd, progen_info, progen_list_from,
    reindex_cmd, rm_cmd as progen_rm_cmd, tree_cmd, BodyDto, NotesDto, ProgenDoctorDto,
    ProgenInfoDto, ProgenListDto, ProgenListItem, ReindexDto, TreeDto,
};
pub use run::{
    action_run_dto, finish_run, format_action_list_human, list_actions_dto, run_cmd, ActionListDto,
    ActionListItem, ActionRunDto, ActionTaskDto, RunOutcome,
};
pub use worktree::{
    add_cmd as worktree_add_cmd, format_worktree_add_human, format_worktree_list_human,
    format_worktree_prune_all_human, format_worktree_prune_human, format_worktree_rm_human,
    list_cmd as worktree_list_cmd, prune_cmd as worktree_prune_cmd, rm_cmd as worktree_rm_cmd,
    worktree_list_dto, worktree_prune_all_dto, worktree_prune_dto, worktree_slot_action_dto,
    WorktreeListDto, WorktreePruneAllDto, WorktreePruneDto, WorktreeSlotActionDto, WorktreeSlotDto,
};
pub use workspace::{
    doctor_cmd, init_cmd, pin_apply_cmd, pin_record_cmd, pin_status_cmd, status_cmd, sync_cmd,
    InitDto, PinApplyDto, PinRecordDto, PinStatusDto, SyncDto,
};
