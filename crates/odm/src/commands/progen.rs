//! `odm progen` handlers — membership + store façade + DTOs.

use std::path::PathBuf;

use odm_core::{
    build_status, path_buf_to_rel, OdmError, PinState, ProgenEntry, StatusSnapshot, Workspace,
};
use odm_git::Git;
use odm_progen::{
    add_progen, context_notes, doctor_progens, format_context_human, format_get_human,
    format_ls_human, get_note, list_notes, one_progen_flag, open_for_id, open_single,
    reindex_for_cli, rm_progen, scoped_from_config, vault_info, ContextHit, GetResult, IndexStats,
    LsHit, ProgenDoctorCheck,
};
use serde::Serialize;

use crate::commands::materialize::{format_progen_add_human, materialize_json_opt};
use crate::ctx::Ctx;
use crate::present::{json_value, NamedMaterialize, NamedOk, Present, Ready};

/// `odm progen list --json` envelope.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProgenListDto {
    pub progens: Vec<ProgenListItem>,
}

/// One Progen row for list JSON (locked field names).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProgenListItem {
    pub name: String,
    pub path: String,
    pub url: Option<String>,
    pub branch: Option<String>,
    pub on_disk: bool,
    pub is_git: bool,
    pub pin_state: Option<PinState>,
}

/// `odm progen info --json` (locked field names).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProgenInfoDto {
    pub name: String,
    pub path: String,
    pub url: Option<String>,
    pub branch: Option<String>,
    pub on_disk: bool,
    pub note_count: usize,
    pub has_obsidian: bool,
    pub abs_path: PathBuf,
}

/// Pure projection: Workspace config ⨝ status snapshot → list DTO.
pub fn progen_list_from(ws: &Workspace, snap: &StatusSnapshot) -> ProgenListDto {
    let progens = ws
        .config
        .progens
        .iter()
        .map(|(name, e)| {
            let st = snap.progens.iter().find(|p| p.name == *name);
            ProgenListItem {
                name: name.clone(),
                path: e.path.clone(),
                url: e.url.clone(),
                branch: e.branch.clone(),
                on_disk: st.map(|s| s.on_disk).unwrap_or(false),
                is_git: st.map(|s| s.is_git).unwrap_or(false),
                pin_state: st.map(|s| s.pin_state),
            }
        })
        .collect();
    ProgenListDto { progens }
}

/// Library entrypoint: observe + progen list DTO.
pub fn list_progens<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
) -> Result<ProgenListDto, OdmError> {
    let snap = build_status(git, ws)?;
    Ok(progen_list_from(ws, &snap))
}

/// Library entrypoint: scoped vault + config → info DTO (bin does not compose scope).
pub fn progen_info(ws: &Workspace, name: &str) -> Result<ProgenInfoDto, OdmError> {
    let entry = ws
        .config
        .progens
        .get(name)
        .ok_or_else(|| OdmError::usage(format!("unknown progen '{name}'")))?;
    let sp = scoped_from_config(&ws.root, &ws.config, name)?;
    let info = vault_info(&sp)?;
    Ok(ProgenInfoDto {
        name: name.into(),
        path: entry.path.clone(),
        url: entry.url.clone(),
        branch: entry.branch.clone(),
        on_disk: info.on_disk,
        note_count: info.note_count,
        has_obsidian: info.has_obsidian,
        abs_path: info.path,
    })
}

/// Human multi-line list from the same DTO as JSON (no dual join).
pub fn format_progen_list_human(dto: &ProgenListDto) -> String {
    if dto.progens.is_empty() {
        return "(no progens)\n".into();
    }
    let mut out = String::new();
    for p in &dto.progens {
        let managed = if p.url.is_some() { "managed" } else { "path" };
        let pin = p.pin_state.map(|s| s.as_str()).unwrap_or("-");
        out.push_str(&format!(
            "{}\t{}\t{managed}\ton_disk={}\tis_git={}\tpin={pin}\n",
            p.name, p.path, p.on_disk, p.is_git
        ));
    }
    out
}

/// Human multi-line info (beside DTO).
pub fn format_progen_info_human(dto: &ProgenInfoDto) -> String {
    let mut out = String::new();
    out.push_str(&format!("name: {}\n", dto.name));
    out.push_str(&format!("path: {}\n", dto.path));
    if let Some(u) = &dto.url {
        out.push_str(&format!("url: {u}\n"));
    }
    out.push_str(&format!("on_disk: {}\n", dto.on_disk));
    out.push_str(&format!("notes: {}\n", dto.note_count));
    out.push_str(&format!("obsidian: {}\n", dto.has_obsidian));
    out.push_str(&format!("abs: {}\n", dto.abs_path.display()));
    out
}

impl Present for ProgenListDto {
    fn to_json(&self) -> Result<serde_json::Value, OdmError> {
        json_value(self)
    }
    fn to_human(&self) -> String {
        format_progen_list_human(self)
    }
}

impl Present for ProgenInfoDto {
    fn to_json(&self) -> Result<serde_json::Value, OdmError> {
        json_value(self)
    }
    fn to_human(&self) -> String {
        format_progen_info_human(self)
    }
}

// --- store DTOs ---

#[derive(Debug, Clone, Serialize)]
pub struct BodyDto {
    pub progen: String,
    pub id: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TreeDto {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BacklinksDto {
    pub backlinks: Vec<LsHit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NotesDto {
    pub notes: Vec<LsHit>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReindexDto {
    pub results: Vec<ReindexItemDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReindexItemDto {
    pub progen: String,
    pub notes: usize,
    pub links: usize,
}

impl From<&IndexStats> for ReindexItemDto {
    fn from(s: &IndexStats) -> Self {
        Self {
            progen: s.progen.clone(),
            notes: s.notes,
            links: s.links,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgenDoctorDto {
    pub ok: bool,
    pub checks: Vec<ProgenDoctorCheck>,
}

// --- handlers ---

pub fn list_cmd(ctx: &Ctx) -> Result<ProgenListDto, OdmError> {
    list_progens(&ctx.git, &ctx.ws)
}

pub fn info_cmd(ctx: &Ctx, name: &str) -> Result<ProgenInfoDto, OdmError> {
    progen_info(&ctx.ws, name)
}

pub fn add_cmd(
    ctx: &mut Ctx,
    name: &str,
    path: &std::path::Path,
    url: Option<String>,
    branch: Option<String>,
    no_clone: bool,
) -> Result<Ready<NamedMaterialize>, OdmError> {
    let rel = path_buf_to_rel(path)?;
    let entry = ProgenEntry {
        path: rel,
        url,
        branch,
        ..Default::default()
    };
    let outcome = add_progen(
        &ctx.git,
        &ctx.ws.root,
        &mut ctx.ws.config,
        name,
        entry,
        no_clone,
    )?;
    let dto = NamedMaterialize::new(name, materialize_json_opt(outcome));
    Ok(Ready::ok(dto, format_progen_add_human(name, outcome)))
}

pub fn rm_cmd(
    ctx: &mut Ctx,
    name: &str,
    delete: bool,
    force: bool,
) -> Result<Ready<NamedOk>, OdmError> {
    rm_progen(
        &ctx.git,
        &ctx.ws.root,
        &mut ctx.ws.config,
        name,
        delete,
        force,
    )?;
    Ok(Ready::ok(
        NamedOk::new(name),
        format!("removed progen {name}"),
    ))
}

pub fn get_cmd(ctx: &Ctx, id: &str) -> Result<Ready<GetResult>, OdmError> {
    let progen = one_progen_flag(
        &ctx.progen,
        "progen get accepts at most one --progen (or use name:id)",
    )?;
    let g = get_note(&ctx.ws, id, progen)?;
    let human = format_get_human(&g);
    Ok(Ready::ok(g, human))
}

pub fn body_cmd(ctx: &Ctx, id: &str) -> Result<Ready<BodyDto>, OdmError> {
    let progen = one_progen_flag(
        &ctx.progen,
        "progen body accepts at most one --progen (or use name:id)",
    )?;
    let g = get_note(&ctx.ws, id, progen)?;
    let mut human = g.body.clone();
    if !human.ends_with('\n') {
        human.push('\n');
    }
    Ok(Ready::ok(
        BodyDto {
            progen: g.progen,
            id: g.id,
            body: g.body,
        },
        human,
    ))
}

pub fn tree_cmd(ctx: &Ctx) -> Result<Ready<TreeDto>, OdmError> {
    let progen = one_progen_flag(&ctx.progen, "progen tree accepts at most one --progen")?;
    let paths = open_single(&ctx.ws, progen)?.tree()?;
    let human = if paths.is_empty() {
        "(no notes)\n".into()
    } else {
        let mut s = String::new();
        for p in &paths {
            s.push_str(p);
            s.push('\n');
        }
        s
    };
    Ok(Ready::ok(TreeDto { paths }, human))
}

pub fn backlinks_cmd(ctx: &Ctx, id: &str) -> Result<Ready<BacklinksDto>, OdmError> {
    let progen = one_progen_flag(
        &ctx.progen,
        "progen backlinks accepts at most one --progen (or use name:id)",
    )?;
    let (store, nid) = open_for_id(&ctx.ws, id, progen)?;
    let hits = store.backlinks(&nid)?;
    let human = format_ls_human(&hits);
    Ok(Ready::ok(BacklinksDto { backlinks: hits }, human))
}

pub fn ls_cmd(ctx: &Ctx) -> Result<Ready<NotesDto>, OdmError> {
    let progen = one_progen_flag(&ctx.progen, "progen ls accepts at most one --progen")?;
    let hits = list_notes(&ctx.ws, progen)?;
    let human = format_ls_human(&hits);
    Ok(Ready::ok(NotesDto { notes: hits }, human))
}

pub fn reindex_cmd(ctx: &Ctx) -> Result<Ready<ReindexDto>, OdmError> {
    let stats = reindex_for_cli(&ctx.ws, &ctx.progen)?;
    let dto = ReindexDto {
        results: stats.iter().map(ReindexItemDto::from).collect(),
    };
    let mut human = String::new();
    for s in &stats {
        human.push_str(&format!(
            "{}\t{} notes\t{} links\n",
            s.progen, s.notes, s.links
        ));
    }
    Ok(Ready::ok(dto, human))
}

pub fn doctor_cmd(ctx: &Ctx) -> Result<Ready<ProgenDoctorDto>, OdmError> {
    let progen = one_progen_flag(&ctx.progen, "progen doctor accepts at most one --progen")?;
    let checks = doctor_progens(&ctx.ws, progen)?;
    let ok = checks.iter().all(|c| c.ok);
    let dto = ProgenDoctorDto { ok, checks };
    let human = if dto.checks.is_empty() {
        "(no progens)\n".into()
    } else {
        let mut s = String::new();
        for c in &dto.checks {
            let mark = if c.ok { "ok" } else { "FAIL" };
            s.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                c.progen, c.id, mark, c.message
            ));
        }
        s
    };
    let exit = if ok { 0 } else { 3 };
    Ok(Ready::with_exit(dto, human, exit))
}

/// `odm context` handler: note neighborhood as a work-package.
pub fn context_cmd(
    ctx: &Ctx,
    id: &str,
    multi_progen_msg: &str,
) -> Result<Ready<ContextHit>, OdmError> {
    let progen = one_progen_flag(&ctx.progen, multi_progen_msg)?;
    let hit = context_notes(&ctx.ws, id, progen)?;
    let human = format_context_human(&hit);
    Ok(Ready::ok(hit, human))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use odm_core::{EntityStatus, PinState, ProgenEntry, WorkspaceConfig};

    #[test]
    fn progen_list_dto_locked_json_shape() {
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
        let ws = Workspace {
            root: PathBuf::from("/tmp/ws"),
            config: WorkspaceConfig {
                progens,
                ..Default::default()
            },
            actions: BTreeMap::new(),
            generators: BTreeMap::new(),
        };
        let snap = StatusSnapshot {
            root: "/tmp/ws".into(),
            projects: vec![],
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
        let dto = progen_list_from(&ws, &snap);
        let v = serde_json::to_value(&dto).unwrap();
        let p = &v["progens"][0];
        assert_eq!(p["name"], "notes");
        assert_eq!(p["path"], "progens/notes");
        assert_eq!(p["on_disk"], true);
        assert_eq!(p["is_git"], false);
        assert_eq!(p["pin_state"], "none");
        assert!(p.get("url").is_some());
        assert!(p.get("branch").is_some());
    }

    #[test]
    fn progen_info_dto_locked_json_shape() {
        let dto = ProgenInfoDto {
            name: "desk".into(),
            path: "progens/desk".into(),
            url: None,
            branch: None,
            on_disk: true,
            note_count: 3,
            has_obsidian: true,
            abs_path: PathBuf::from("/tmp/ws/progens/desk"),
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["name"], "desk");
        assert_eq!(v["path"], "progens/desk");
        assert_eq!(v["on_disk"], true);
        assert_eq!(v["note_count"], 3);
        assert_eq!(v["has_obsidian"], true);
        assert_eq!(v["abs_path"], "/tmp/ws/progens/desk");
    }
}
