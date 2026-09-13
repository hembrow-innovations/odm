use odm_git::Git;
use serde::Serialize;

use crate::config::{CheckoutMode, Workspace};
use crate::error::OdmError;
use crate::inventory::observe_project_worktrees_soft;
use crate::pin::load_pin;
use crate::worktree::{WorktreeOrphanInfo, WorktreeSlotInfo};

/// `odm status --json` snapshot.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StatusSnapshot {
    pub root: String,
    pub projects: Vec<EntityStatus>,
    pub progens: Vec<EntityStatus>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EntityStatus {
    pub name: String,
    pub path: String,
    pub url: Option<String>,
    pub managed: bool,
    pub on_disk: bool,
    pub is_git: bool,
    pub head: Option<String>,
    pub pin_rev: Option<String>,
    pub pin_state: PinState,
    pub dirty: Option<bool>,
    /// Registered worktree slots for Projects only (`None` on Progens → omitted from JSON).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_slots: Option<Vec<WorktreeSlotInfo>>,
    /// Orphan slot dirs for Projects only (`None` on Progens → omitted from JSON).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_orphans: Option<Vec<WorktreeOrphanInfo>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinSource {
    Unmanaged,
    LockFile,
    Gitlink,
}

/// Classify pin authority from membership and checkout mode.
pub fn pin_source(managed: bool, checkout: CheckoutMode) -> PinSource {
    if !managed {
        PinSource::Unmanaged
    } else if checkout == CheckoutMode::Gitlink {
        PinSource::Gitlink
    } else {
        PinSource::LockFile
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PinState {
    None,
    MissingPath,
    Unpinned,
    InSync,
    Drift,
    MissingPinFile,
}

impl PinState {
    /// Locked snake_case label for JSON `pin_state` / pin status `state`.
    pub fn as_str(self) -> &'static str {
        match self {
            PinState::None => "none",
            PinState::MissingPath => "missing_path",
            PinState::Unpinned => "unpinned",
            PinState::InSync => "in_sync",
            PinState::Drift => "drift",
            PinState::MissingPinFile => "missing_pin_file",
        }
    }
}

/// Build Workspace status snapshot. Does not fetch.
pub fn build_status<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
) -> Result<StatusSnapshot, OdmError> {
    let pin = load_pin(&ws.root)?;
    let obs = crate::observation::observe_workspace(git, &ws.root, &ws.config, pin.as_ref())?;
    let mut snap = status_from_observation(&obs);
    for p in &mut snap.projects {
        let inv = observe_project_worktrees_soft(git, ws, &p.name);
        p.worktree_slots = Some(inv.slots);
        p.worktree_orphans = Some(inv.orphans);
    }
    Ok(snap)
}

/// Pure projection: observation → status snapshot.
///
/// `worktree_slots` / `worktree_orphans` stay `None` here;
/// [`build_status`] fills them from inventory.
pub fn status_from_observation(obs: &crate::observation::WorkspaceObservation) -> StatusSnapshot {
    StatusSnapshot {
        root: obs.root.clone(),
        projects: obs.projects.iter().map(entity_status_from_obs).collect(),
        progens: obs.progens.iter().map(entity_status_from_obs).collect(),
    }
}

fn entity_status_from_obs(e: &crate::observation::EntityObservation) -> EntityStatus {
    EntityStatus {
        name: e.name.clone(),
        path: e.path.clone(),
        url: e.url.clone(),
        managed: e.managed,
        on_disk: e.on_disk,
        is_git: e.is_git,
        head: e.head.clone(),
        pin_rev: e.pin_rev.clone(),
        pin_state: e.pin_state,
        dirty: e.dirty,
        worktree_slots: None,
        worktree_orphans: None,
    }
}

/// Single source of truth for pin drift labels.
///
/// Unmanaged → None. LockFile: `!pin_present` → MissingPinFile. Gitlink never
/// yields MissingPinFile. Then `pin_rev` none → Unpinned; `!on_disk` → MissingPath;
/// head==pin_rev → InSync; else Drift.
pub fn compute_pin_state(
    source: PinSource,
    pin_present: bool,
    on_disk: bool,
    pin_rev: Option<&str>,
    head: Option<&str>,
) -> PinState {
    match source {
        PinSource::Unmanaged => PinState::None,
        PinSource::LockFile if !pin_present => PinState::MissingPinFile,
        PinSource::LockFile | PinSource::Gitlink => sha_pin_state(on_disk, pin_rev, head),
    }
}

fn sha_pin_state(on_disk: bool, pin_rev: Option<&str>, head: Option<&str>) -> PinState {
    if pin_rev.is_none() {
        return PinState::Unpinned;
    }
    if !on_disk {
        return PinState::MissingPath;
    }
    match (pin_rev, head) {
        (Some(p), Some(h)) if p == h => PinState::InSync,
        (Some(_), _) => PinState::Drift,
        _ => PinState::Unpinned,
    }
}

/// Human-readable multi-line summary.
pub fn format_status_human(snap: &StatusSnapshot) -> String {
    let mut out = String::new();
    out.push_str(&format!("Workspace: {}\n", snap.root));
    if snap.projects.is_empty() && snap.progens.is_empty() {
        out.push_str("(no projects or progens)\n");
        return out;
    }
    if !snap.projects.is_empty() {
        out.push_str("\nProjects:\n");
        for e in &snap.projects {
            out.push_str(&format_entity_line(e));
        }
    }
    if !snap.progens.is_empty() {
        out.push_str("\nProgens:\n");
        for e in &snap.progens {
            out.push_str(&format_entity_line(e));
        }
    }
    out
}

fn format_entity_line(e: &EntityStatus) -> String {
    let kind = if e.managed { "managed" } else { "path" };
    let disk = if e.on_disk {
        if e.is_git {
            "git"
        } else {
            "disk"
        }
    } else {
        "missing"
    };
    let pin = match e.pin_state {
        PinState::None => "-",
        other => other.as_str(),
    };
    let dirty = match e.dirty {
        Some(true) => " dirty",
        Some(false) => " clean",
        None => "",
    };
    let mut line = format!(
        "  {}\t{}\t{}\t{}\tpin={}{}\n",
        e.name, e.path, kind, disk, pin, dirty
    );
    if let Some(slots) = &e.worktree_slots {
        if !slots.is_empty() {
            let names: Vec<String> = slots
                .iter()
                .map(|s| {
                    if s.dirty == Some(true) {
                        format!("{} dirty", s.name)
                    } else {
                        s.name.clone()
                    }
                })
                .collect();
            line.push_str(&format!("    worktrees: {}\n", names.join(", ")));
        }
    }
    if let Some(orphans) = &e.worktree_orphans {
        if !orphans.is_empty() {
            let names: Vec<&str> = orphans.iter().map(|o| o.name.as_str()).collect();
            line.push_str(&format!("    orphans: {}\n", names.join(", ")));
        }
    }
    line
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
