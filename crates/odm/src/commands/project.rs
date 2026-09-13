//! `odm project` handlers — list/info/add/rm + DTOs.

use odm_core::{
    build_status, load_pin, observe_project_worktrees_soft, observe_workspace, path_buf_to_rel,
    project_add, project_git, project_rm, CheckoutMode, EntityObservation, OdmError, PinState,
    ProjectEntry, StatusSnapshot, Workspace, WorktreeOrphanInfo, WorktreeSlotInfo,
};
use odm_git::Git;
use serde::Serialize;

use crate::commands::materialize::{
    format_project_add_human, materialize_add_json, unstage_and_rewrite_gitlink,
};
use crate::ctx::Ctx;
use crate::present::{json_value, NamedMaterialize, NamedOk, Present, Ready};

/// `odm project list --json` envelope.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProjectListDto {
    pub projects: Vec<ProjectListItem>,
}

/// One Project row for list JSON (locked field names).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProjectListItem {
    pub name: String,
    pub path: String,
    pub url: Option<String>,
    pub branch: Option<String>,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    pub on_disk: bool,
    pub is_git: bool,
    pub pin_state: Option<PinState>,
}

/// `odm project info --json` (locked field names).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProjectInfoDto {
    pub name: String,
    pub path: String,
    pub url: Option<String>,
    pub branch: Option<String>,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    pub on_disk: bool,
    pub is_git: bool,
    pub head: Option<String>,
    pub origin: Option<String>,
    pub dirty: Option<bool>,
    pub pin_rev: Option<String>,
    pub pin_state: PinState,
    /// Registered worktree slots (`name` + `path`); always present, empty when none.
    pub worktree_slots: Vec<WorktreeSlotInfo>,
    /// Orphan slot dirs under `worktrees/<project>/` (`name` + `path`); always present.
    pub worktree_orphans: Vec<WorktreeOrphanInfo>,
}

/// Pure projection: Workspace config ⨝ status snapshot → list DTO.
pub fn project_list_from(ws: &Workspace, snap: &StatusSnapshot) -> ProjectListDto {
    let projects = ws
        .config
        .projects
        .iter()
        .map(|(name, e)| {
            let st = snap.projects.iter().find(|p| p.name == *name);
            ProjectListItem {
                name: name.clone(),
                path: e.path.clone(),
                url: e.url.clone(),
                branch: e.branch.clone(),
                type_: e.type_.clone(),
                on_disk: st.map(|s| s.on_disk).unwrap_or(false),
                is_git: st.map(|s| s.is_git).unwrap_or(false),
                pin_state: st.map(|s| s.pin_state),
            }
        })
        .collect();
    ProjectListDto { projects }
}

/// Pure projection: config entry ⨝ observation row → info DTO.
/// Origin comes from observation (no second git query).
pub fn project_info_from(
    name: &str,
    path: &str,
    url: Option<&str>,
    branch: Option<&str>,
    type_: Option<&str>,
    obs: &EntityObservation,
) -> ProjectInfoDto {
    ProjectInfoDto {
        name: name.into(),
        path: path.into(),
        url: url.map(str::to_string),
        branch: branch.map(str::to_string),
        type_: type_.map(str::to_string),
        on_disk: obs.on_disk,
        is_git: obs.is_git,
        head: obs.head.clone(),
        origin: if obs.is_git { obs.origin.clone() } else { None },
        dirty: obs.dirty,
        pin_rev: obs.pin_rev.clone(),
        pin_state: obs.pin_state,
        // Filled by [`project_info`] via soft-fail `worktree_list`.
        worktree_slots: vec![],
        worktree_orphans: vec![],
    }
}

/// Library entrypoint: observe + project list DTO.
pub fn list_projects<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
) -> Result<ProjectListDto, OdmError> {
    let snap = build_status(git, ws)?;
    Ok(project_list_from(ws, &snap))
}

/// Library entrypoint: observe once + project info DTO (origin from observation).
pub fn project_info<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
    name: &str,
) -> Result<ProjectInfoDto, OdmError> {
    let entry = ws
        .config
        .projects
        .get(name)
        .ok_or_else(|| OdmError::usage(format!("unknown project '{name}'")))?;
    let pin = load_pin(&ws.root)?;
    let obs = observe_workspace(git, &ws.root, &ws.config, pin.as_ref())?;
    let st = obs
        .projects
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| OdmError::usage(format!("unknown project '{name}'")))?;
    let mut dto = project_info_from(
        name,
        &entry.path,
        entry.url.as_deref(),
        entry.branch.as_deref(),
        entry.type_.as_deref(),
        st,
    );
    let inv = observe_project_worktrees_soft(git, ws, name);
    dto.worktree_slots = inv.slots;
    dto.worktree_orphans = inv.orphans;
    Ok(dto)
}

/// Human multi-line list from the same DTO as JSON (no dual join).
pub fn format_project_list_human(dto: &ProjectListDto) -> String {
    if dto.projects.is_empty() {
        return "(no projects)\n".into();
    }
    let mut out = String::new();
    for p in &dto.projects {
        let managed = if p.url.is_some() { "managed" } else { "path" };
        let pin = p.pin_state.map(|s| s.as_str()).unwrap_or("-");
        out.push_str(&format!(
            "{}\t{}\t{managed}\ton_disk={}\tis_git={}\tpin={pin}\n",
            p.name, p.path, p.on_disk, p.is_git
        ));
    }
    out
}

impl Present for ProjectListDto {
    fn to_json(&self) -> Result<serde_json::Value, OdmError> {
        json_value(self)
    }
    fn to_human(&self) -> String {
        format_project_list_human(self)
    }
}

impl Present for ProjectInfoDto {
    fn to_json(&self) -> Result<serde_json::Value, OdmError> {
        json_value(self)
    }
    fn to_human(&self) -> String {
        format_project_info_human(self)
    }
}

/// Handler: list projects.
pub fn list_cmd(ctx: &Ctx) -> Result<ProjectListDto, OdmError> {
    list_projects(&ctx.git, &ctx.ws)
}

/// Handler: project info.
pub fn info_cmd(ctx: &Ctx, name: &str) -> Result<ProjectInfoDto, OdmError> {
    project_info(&ctx.git, &ctx.ws, name)
}

/// Handler: project add → named materialize envelope.
pub fn add_cmd(
    ctx: &mut Ctx,
    name: &str,
    path: &std::path::Path,
    url: Option<String>,
    branch: Option<String>,
    type_: Option<String>,
    no_clone: bool,
    gitlink: bool,
) -> Result<Ready<NamedMaterialize>, OdmError> {
    let rel = path_buf_to_rel(path)?;
    let entry = ProjectEntry {
        path: rel,
        url,
        branch,
        type_,
        checkout: if gitlink {
            CheckoutMode::Gitlink
        } else {
            CheckoutMode::Clone
        },
    };
    let outcome = project_add(
        &ctx.git,
        &ctx.ws.root,
        &mut ctx.ws.config,
        name,
        entry,
        no_clone,
    )?;
    let dto = NamedMaterialize::new(name, materialize_add_json(outcome, gitlink));
    Ok(Ready::ok(dto, format_project_add_human(name, outcome)))
}

/// Handler: project rm → named ok envelope.
pub fn rm_cmd(
    ctx: &mut Ctx,
    name: &str,
    delete: bool,
    force: bool,
) -> Result<Ready<NamedOk>, OdmError> {
    let gitlink_path = ctx
        .ws
        .config
        .projects
        .get(name)
        .and_then(|e| (e.checkout == CheckoutMode::Gitlink).then(|| e.path.clone()));
    project_rm(
        &ctx.git,
        &ctx.ws.root,
        &mut ctx.ws.config,
        name,
        delete,
        force,
    )?;
    if let Some(rel) = gitlink_path {
        unstage_and_rewrite_gitlink(&ctx.git, &ctx.ws.root, &ctx.ws.config, &rel)?;
    }
    Ok(Ready::ok(
        NamedOk::new(name),
        format!("removed project {name}"),
    ))
}

/// Handler: project git passthrough — raw exit only (stdio already inherited).
pub fn git_cmd(
    ctx: &Ctx,
    name: &str,
    git_args: &[String],
) -> Result<std::process::ExitStatus, OdmError> {
    project_git(&ctx.git, &ctx.ws, name, git_args, ctx.wt.as_deref())
}

/// Human multi-line info (beside DTO).
pub fn format_project_info_human(dto: &ProjectInfoDto) -> String {
    let mut out = String::new();
    out.push_str(&format!("name: {}\n", dto.name));
    out.push_str(&format!("path: {}\n", dto.path));
    if let Some(u) = &dto.url {
        out.push_str(&format!("url: {u}\n"));
    }
    if let Some(b) = &dto.branch {
        out.push_str(&format!("branch: {b}\n"));
    }
    if let Some(t) = &dto.type_ {
        out.push_str(&format!("type: {t}\n"));
    }
    out.push_str(&format!("on_disk: {}\n", dto.on_disk));
    out.push_str(&format!("is_git: {}\n", dto.is_git));
    if let Some(h) = &dto.head {
        out.push_str(&format!("head: {h}\n"));
    }
    if let Some(o) = &dto.origin {
        out.push_str(&format!("origin: {o}\n"));
    }
    out.push_str(&format!("pin_state: {:?}\n", dto.pin_state));
    if !dto.worktree_slots.is_empty() {
        let names: Vec<String> = dto
            .worktree_slots
            .iter()
            .map(|s| {
                if s.dirty == Some(true) {
                    format!("{} dirty", s.name)
                } else {
                    s.name.clone()
                }
            })
            .collect();
        out.push_str(&format!("worktrees: {}\n", names.join(", ")));
    }
    if !dto.worktree_orphans.is_empty() {
        let names: Vec<&str> = dto
            .worktree_orphans
            .iter()
            .map(|o| o.name.as_str())
            .collect();
        out.push_str(&format!("orphans: {}\n", names.join(", ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use odm_core::{
        EntityStatus, PinState, ProjectEntry, WorkspaceConfig, WorktreeOrphanInfo, WorktreeSlotInfo,
    };

    fn ws_one(name: &str, path: &str) -> Workspace {
        let mut projects = BTreeMap::new();
        projects.insert(
            name.into(),
            ProjectEntry {
                path: path.into(),
                url: Some("https://example.com/a.git".into()),
                branch: Some("main".into()),
                type_: Some("app".into()),
                ..Default::default()
            },
        );
        Workspace {
            root: PathBuf::from("/tmp/ws"),
            config: WorkspaceConfig {
                projects,
                ..Default::default()
            },
            actions: BTreeMap::new(),
            generators: BTreeMap::new(),
        }
    }

    fn snap_one(name: &str, path: &str) -> StatusSnapshot {
        StatusSnapshot {
            root: "/tmp/ws".into(),
            projects: vec![EntityStatus {
                name: name.into(),
                path: path.into(),
                url: Some("https://example.com/a.git".into()),
                managed: true,
                on_disk: true,
                is_git: true,
                head: Some("abc".into()),
                pin_rev: None,
                pin_state: PinState::MissingPinFile,
                dirty: Some(false),
                worktree_slots: None,
                worktree_orphans: None,
            }],
            progens: vec![],
        }
    }

    #[test]
    fn project_list_dto_locked_json_shape() {
        let ws = ws_one("alpha", "projects/alpha");
        let snap = snap_one("alpha", "projects/alpha");
        let dto = project_list_from(&ws, &snap);
        let v = serde_json::to_value(&dto).unwrap();
        let p = &v["projects"][0];
        assert_eq!(p["name"], "alpha");
        assert_eq!(p["path"], "projects/alpha");
        assert_eq!(p["url"], "https://example.com/a.git");
        assert_eq!(p["branch"], "main");
        assert_eq!(p["type"], "app");
        assert_eq!(p["on_disk"], true);
        assert_eq!(p["is_git"], true);
        assert_eq!(p["pin_state"], "missing_pin_file");
        // envelope key
        assert!(v.get("projects").unwrap().is_array());
    }

    #[test]
    fn project_list_human_from_same_dto() {
        let ws = ws_one("alpha", "projects/alpha");
        let snap = snap_one("alpha", "projects/alpha");
        let dto = project_list_from(&ws, &snap);
        let human = format_project_list_human(&dto);
        assert!(
            human.contains(
                "alpha\tprojects/alpha\tmanaged\ton_disk=true\tis_git=true\tpin=missing_pin_file\n"
            ),
            "{human}"
        );
        assert_eq!(
            format_project_list_human(&ProjectListDto { projects: vec![] }),
            "(no projects)\n"
        );
    }

    #[test]
    fn project_list_missing_status_defaults() {
        let ws = ws_one("ghost", "projects/ghost");
        let snap = StatusSnapshot {
            root: "/tmp/ws".into(),
            projects: vec![],
            progens: vec![],
        };
        let dto = project_list_from(&ws, &snap);
        assert_eq!(dto.projects.len(), 1);
        assert!(!dto.projects[0].on_disk);
        assert!(!dto.projects[0].is_git);
        assert!(dto.projects[0].pin_state.is_none());
    }

    #[test]
    fn project_info_dto_uses_observation_origin() {
        let obs = EntityObservation {
            name: "alpha".into(),
            path: "projects/alpha".into(),
            url: Some("https://example.com/a.git".into()),
            managed: true,
            abs_path: Some(PathBuf::from("/tmp/ws/projects/alpha")),
            resolve_error: None,
            on_disk: true,
            is_git: true,
            head: Some("deadbeef".into()),
            origin: Some("https://example.com/a.git".into()),
            dirty: Some(false),
            pin_rev: None,
            pin_state: PinState::MissingPinFile,
        };
        let dto = project_info_from(
            "alpha",
            "projects/alpha",
            Some("https://example.com/a.git"),
            Some("main"),
            Some("app"),
            &obs,
        );
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["name"], "alpha");
        assert_eq!(v["path"], "projects/alpha");
        assert_eq!(v["type"], "app");
        assert_eq!(v["origin"], "https://example.com/a.git");
        assert_eq!(v["head"], "deadbeef");
        assert_eq!(v["pin_state"], "missing_pin_file");
        assert_eq!(v["dirty"], false);
    }

    #[test]
    fn project_info_origin_none_when_not_git() {
        let obs = EntityObservation {
            name: "local".into(),
            path: "projects/local".into(),
            url: None,
            managed: false,
            abs_path: Some(PathBuf::from("/tmp/ws/projects/local")),
            resolve_error: None,
            on_disk: true,
            is_git: false,
            head: None,
            origin: Some("should-ignore".into()),
            dirty: None,
            pin_rev: None,
            pin_state: PinState::None,
        };
        let dto = project_info_from("local", "projects/local", None, None, None, &obs);
        assert!(dto.origin.is_none());
    }

    #[test]
    fn project_info_dto_worktree_slots_always_present_empty_by_default() {
        let obs = EntityObservation {
            name: "alpha".into(),
            path: "projects/alpha".into(),
            url: None,
            managed: false,
            abs_path: Some(PathBuf::from("/tmp/ws/projects/alpha")),
            resolve_error: None,
            on_disk: true,
            is_git: true,
            head: Some("abc".into()),
            origin: None,
            dirty: Some(false),
            pin_rev: None,
            pin_state: PinState::None,
        };
        let dto = project_info_from("alpha", "projects/alpha", None, None, None, &obs);
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["worktree_slots"], serde_json::json!([]));
        assert!(dto.worktree_slots.is_empty());
        assert_eq!(v["worktree_orphans"], serde_json::json!([]));
        assert!(dto.worktree_orphans.is_empty());
    }

    #[test]
    fn project_info_dto_worktree_orphans_json_name_and_path_no_dirty() {
        let mut dto = project_info_from(
            "alpha",
            "projects/alpha",
            None,
            None,
            None,
            &EntityObservation {
                name: "alpha".into(),
                path: "projects/alpha".into(),
                url: None,
                managed: false,
                abs_path: None,
                resolve_error: None,
                on_disk: true,
                is_git: true,
                head: None,
                origin: None,
                dirty: None,
                pin_rev: None,
                pin_state: PinState::None,
            },
        );
        dto.worktree_orphans = vec![WorktreeOrphanInfo {
            name: "stale".into(),
            path: "worktrees/alpha/stale".into(),
        }];
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["worktree_orphans"][0]["name"], "stale");
        assert_eq!(v["worktree_orphans"][0]["path"], "worktrees/alpha/stale");
        assert!(v["worktree_orphans"][0].get("dirty").is_none());
    }

    #[test]
    fn project_info_dto_worktree_slots_json_name_and_path() {
        let mut dto = project_info_from(
            "alpha",
            "projects/alpha",
            None,
            None,
            None,
            &EntityObservation {
                name: "alpha".into(),
                path: "projects/alpha".into(),
                url: None,
                managed: false,
                abs_path: None,
                resolve_error: None,
                on_disk: true,
                is_git: true,
                head: None,
                origin: None,
                dirty: None,
                pin_rev: None,
                pin_state: PinState::None,
            },
        );
        dto.worktree_slots = vec![
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
        ];
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["worktree_slots"][0]["name"], "a");
        assert_eq!(v["worktree_slots"][0]["path"], "worktrees/alpha/a");
        assert_eq!(v["worktree_slots"][0]["dirty"], false);
        assert_eq!(v["worktree_slots"][1]["name"], "b");
        assert_eq!(v["worktree_slots"][1]["path"], "worktrees/alpha/b");
        assert_eq!(v["worktree_slots"][1]["dirty"], true);
    }

    #[test]
    fn format_project_info_human_shows_slots_when_non_empty() {
        let dto = ProjectInfoDto {
            name: "alpha".into(),
            path: "projects/alpha".into(),
            url: None,
            branch: None,
            type_: None,
            on_disk: true,
            is_git: true,
            head: None,
            origin: None,
            dirty: None,
            pin_rev: None,
            pin_state: PinState::None,
            worktree_slots: vec![
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
            ],
            worktree_orphans: vec![],
        };
        let human = format_project_info_human(&dto);
        assert!(human.contains("worktrees: a, b dirty"), "{human}");
    }

    #[test]
    fn format_project_info_human_silent_when_slots_empty() {
        let dto = ProjectInfoDto {
            name: "alpha".into(),
            path: "projects/alpha".into(),
            url: None,
            branch: None,
            type_: None,
            on_disk: true,
            is_git: true,
            head: None,
            origin: None,
            dirty: None,
            pin_rev: None,
            pin_state: PinState::None,
            worktree_slots: vec![],
            worktree_orphans: vec![],
        };
        let human = format_project_info_human(&dto);
        assert!(!human.contains("worktrees"), "{human}");
        assert!(!human.contains("orphans"), "{human}");
    }

    #[test]
    fn format_project_info_human_shows_orphans_when_non_empty() {
        let dto = ProjectInfoDto {
            name: "alpha".into(),
            path: "projects/alpha".into(),
            url: None,
            branch: None,
            type_: None,
            on_disk: true,
            is_git: true,
            head: None,
            origin: None,
            dirty: None,
            pin_rev: None,
            pin_state: PinState::None,
            worktree_slots: vec![],
            worktree_orphans: vec![
                WorktreeOrphanInfo {
                    name: "stale".into(),
                    path: "worktrees/alpha/stale".into(),
                },
                WorktreeOrphanInfo {
                    name: "other".into(),
                    path: "worktrees/alpha/other".into(),
                },
            ],
        };
        let human = format_project_info_human(&dto);
        assert!(human.contains("orphans: stale, other"), "{human}");
    }
}
