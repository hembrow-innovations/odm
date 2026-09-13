//! `odm init` / `sync` / `pin` / `status` / `doctor` handlers + DTOs.

use std::path::PathBuf;

use odm_core::{
    format_doctor_human, format_status_human, init_workspace, pin_apply, pin_record, pin_status,
    run_doctor, sync_managed, InitOptions, InitResult, OdmError, PinApplyResult, PinRecordResult,
    PinStatusReport, StatusSnapshot,
};
use serde::Serialize;

use crate::commands::materialize::{materialize_json, materialize_sync_human};
use crate::ctx::Ctx;
use crate::present::Ready;

// --- init ---

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InitDto {
    pub root: PathBuf,
    pub git: bool,
}

impl From<InitResult> for InitDto {
    fn from(r: InitResult) -> Self {
        Self {
            root: r.root,
            git: r.git,
        }
    }
}

pub fn init_cmd(
    path: Option<PathBuf>,
    no_git: bool,
    interactive: bool,
) -> Result<Ready<InitDto>, OdmError> {
    if interactive {
        return Err(OdmError::not_implemented("init --interactive"));
    }
    let target = path.unwrap_or_else(|| PathBuf::from("."));
    let res = init_workspace(InitOptions {
        path: target,
        no_git,
        name: None,
    })?;
    let dto = InitDto::from(res);
    let human = format!(
        "Initialized Workspace at {} (git: {})\n",
        dto.root.display(),
        dto.git
    );
    Ok(Ready::ok(dto, human))
}

// --- sync ---

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SyncDto {
    pub ok: bool,
    pub results: Vec<SyncItemDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SyncItemDto {
    pub name: String,
    pub materialized: &'static str,
    pub fetched: bool,
    pub head: Option<String>,
}

pub fn sync_cmd(ctx: &Ctx, names: &[String]) -> Result<Ready<SyncDto>, OdmError> {
    let results = sync_managed(&ctx.git, &ctx.ws.root, &ctx.ws.config, names)?;
    let items: Vec<SyncItemDto> = results
        .iter()
        .map(|r| SyncItemDto {
            name: r.name.clone(),
            materialized: materialize_json(r.materialized),
            fetched: r.fetched,
            head: r.head.clone(),
        })
        .collect();
    let human = if items.is_empty() {
        "(no managed entries)\n".into()
    } else {
        let mut s = String::new();
        for r in &results {
            s.push_str(&format!(
                "{}\t{}\tfetched\n",
                r.name,
                materialize_sync_human(r.materialized)
            ));
        }
        s
    };
    Ok(Ready::ok(
        SyncDto {
            ok: true,
            results: items,
        },
        human,
    ))
}

// --- pin ---

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PinApplyDto {
    pub results: Vec<PinApplyItemDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PinApplyItemDto {
    pub name: String,
    pub status: String,
    pub rev: Option<String>,
    pub detached: bool,
}

impl From<&PinApplyResult> for PinApplyItemDto {
    fn from(r: &PinApplyResult) -> Self {
        Self {
            name: r.name.clone(),
            status: r.status.clone(),
            rev: r.rev.clone(),
            detached: r.detached,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PinRecordDto {
    pub results: Vec<PinRecordItemDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PinRecordItemDto {
    pub name: String,
    pub status: String,
    pub rev: Option<String>,
}

impl From<&PinRecordResult> for PinRecordItemDto {
    fn from(r: &PinRecordResult) -> Self {
        Self {
            name: r.name.clone(),
            status: r.status.clone(),
            rev: r.rev.clone(),
        }
    }
}

pub fn pin_record_cmd(
    ctx: &Ctx,
    names: &[String],
    force: bool,
) -> Result<Ready<PinRecordDto>, OdmError> {
    let results = pin_record(&ctx.git, &ctx.ws.root, &ctx.ws.config, names, force)?;
    let dto = PinRecordDto {
        results: results.iter().map(PinRecordItemDto::from).collect(),
    };
    if !results.is_empty() {
        eprintln!("parent index is dirty until you commit");
    }
    let human = if results.is_empty() {
        "(nothing to record)\n".into()
    } else {
        let mut s = String::new();
        for r in &results {
            s.push_str(&format!(
                "{}\t{}\t{}\n",
                r.name,
                r.status,
                r.rev.as_deref().unwrap_or("-"),
            ));
        }
        s
    };
    Ok(Ready::ok(dto, human))
}

pub fn pin_apply_cmd(
    ctx: &Ctx,
    names: &[String],
    force: bool,
) -> Result<Ready<PinApplyDto>, OdmError> {
    let results = pin_apply(&ctx.git, &ctx.ws.root, &ctx.ws.config, names, force)?;
    let dto = PinApplyDto {
        results: results.iter().map(PinApplyItemDto::from).collect(),
    };
    let human = if results.is_empty() {
        "(nothing to apply)\n".into()
    } else {
        let mut s = String::new();
        for r in &results {
            let det = if r.detached { "\tdetached HEAD" } else { "" };
            s.push_str(&format!(
                "{}\t{}\t{}{}\n",
                r.name,
                r.status,
                r.rev.as_deref().unwrap_or("-"),
                det
            ));
        }
        s.push_str("applied (detached HEAD)\n");
        s
    };
    Ok(Ready::ok(dto, human))
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PinStatusDto {
    pub pin_file: String,
    pub present: bool,
    pub entries: Vec<PinStatusItemDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PinStatusItemDto {
    pub name: String,
    pub pin_rev: Option<String>,
    pub head: Option<String>,
    pub state: String,
}

impl From<&PinStatusReport> for PinStatusDto {
    fn from(r: &PinStatusReport) -> Self {
        Self {
            pin_file: r.pin_file.clone(),
            present: r.present,
            entries: r
                .entries
                .iter()
                .map(|e| PinStatusItemDto {
                    name: e.name.clone(),
                    pin_rev: e.pin_rev.clone(),
                    head: e.head.clone(),
                    state: e.state.clone(),
                })
                .collect(),
        }
    }
}

pub fn pin_status_cmd(ctx: &Ctx, names: &[String]) -> Result<Ready<PinStatusDto>, OdmError> {
    let report = pin_status(&ctx.git, &ctx.ws.root, &ctx.ws.config, names)?;
    let dto = PinStatusDto::from(&report);
    let human = if report.entries.is_empty() {
        format!(
            "pin file present: {}\n(no managed entries)\n",
            report.present
        )
    } else {
        let mut s = String::new();
        for e in &report.entries {
            s.push_str(&format!(
                "{}\t{}\t{}\n",
                e.name,
                e.state,
                e.pin_rev.as_deref().unwrap_or("-")
            ));
        }
        s
    };
    Ok(Ready::ok(dto, human))
}

// --- status / doctor ---

pub fn status_cmd(ctx: &Ctx) -> Result<Ready<StatusSnapshot>, OdmError> {
    let snap = odm_core::build_status(&ctx.git, &ctx.ws)?;
    let human = format_status_human(&snap);
    Ok(Ready::ok(snap, human))
}

pub fn doctor_cmd(ctx: &Ctx, fix: bool) -> Result<Ready<odm_core::DoctorReport>, OdmError> {
    let report = run_doctor(&ctx.git, &ctx.ws, fix)?;
    let exit = if report.ok { 0 } else { 3 };
    let human = format_doctor_human(&report);
    Ok(Ready::with_exit(report, human, exit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_dto_json_shape() {
        let dto = InitDto {
            root: PathBuf::from("/tmp/ws"),
            git: true,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["root"], "/tmp/ws");
        assert_eq!(v["git"], true);
    }

    #[test]
    fn sync_dto_json_shape() {
        let dto = SyncDto {
            ok: true,
            results: vec![SyncItemDto {
                name: "alpha".into(),
                materialized: "already_present",
                fetched: true,
                head: Some("abc".into()),
            }],
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["results"][0]["name"], "alpha");
        assert_eq!(v["results"][0]["materialized"], "already_present");
        assert_eq!(v["results"][0]["fetched"], true);
        assert_eq!(v["results"][0]["head"], "abc");
    }

    #[test]
    fn pin_record_dto_json_shape() {
        let dto = PinRecordDto {
            results: vec![PinRecordItemDto {
                name: "nested".into(),
                status: "recorded".into(),
                rev: Some("dead".into()),
            }],
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["results"][0]["name"], "nested");
        assert_eq!(v["results"][0]["status"], "recorded");
        assert_eq!(v["results"][0]["rev"], "dead");
        assert!(v["results"][0].get("detached").is_none());
    }

    #[test]
    fn pin_apply_dto_json_shape() {
        let dto = PinApplyDto {
            results: vec![PinApplyItemDto {
                name: "alpha".into(),
                status: "applied".into(),
                rev: Some("dead".into()),
                detached: true,
            }],
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["results"][0]["name"], "alpha");
        assert_eq!(v["results"][0]["status"], "applied");
        assert_eq!(v["results"][0]["rev"], "dead");
        assert_eq!(v["results"][0]["detached"], true);
    }

    #[test]
    fn pin_status_dto_json_shape() {
        let dto = PinStatusDto {
            pin_file: ".odm/odm.lock.yaml".into(),
            present: true,
            entries: vec![PinStatusItemDto {
                name: "alpha".into(),
                pin_rev: Some("a".into()),
                head: Some("b".into()),
                state: "drift".into(),
            }],
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["pin_file"], ".odm/odm.lock.yaml");
        assert_eq!(v["present"], true);
        assert_eq!(v["entries"][0]["state"], "drift");
    }
}
