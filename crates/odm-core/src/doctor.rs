use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use odm_git::{Git, GitlinkRecord};
use serde::Serialize;

use crate::checkout::all_managed;
use crate::config::{odm_dir, pin_path, CheckoutMode, Workspace};
use crate::error::OdmError;
use crate::gitignore::{
    ancestor_gitignore_has_drift, apply_managed_gitignore, workspace_gitignore_has_drift,
};
use crate::gitmodules::{gitmodules_has_drift, rewrite_gitmodules};
use crate::observation::{observe_workspace, EntityObservation};
use crate::paths::abs_checkout;
use crate::pin::{is_full_sha, load_pin, parse_pin_yaml};
use crate::url_match::urls_match_with_root;

/// `odm doctor --json` report.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DoctorReport {
    pub ok: bool,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DoctorCheck {
    pub id: String,
    pub status: CheckStatus,
    pub message: String,
    pub fixable: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

/// Run doctor checks. When `fix` is true, apply mechanical repairs then re-check.
pub fn run_doctor<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
    fix: bool,
) -> Result<DoctorReport, OdmError> {
    if fix {
        apply_fixes(git, ws)?;
    }
    let checks = collect_checks(git, ws)?;
    let ok = !checks.iter().any(|c| c.status == CheckStatus::Fail);
    Ok(DoctorReport { ok, checks })
}

fn apply_fixes<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
) -> Result<(), OdmError> {
    ensure_odm_layout(&ws.root)?;
    if ws.config.manage_gitignore() && git.is_repo(&ws.root).unwrap_or(false) {
        apply_managed_gitignore(&ws.root, &ws.config)?;
    } else if ws.config.manage_gitignore() {
        // Still rewrite ignore files if manage is on (doctor --fix allowlist).
        apply_managed_gitignore(&ws.root, &ws.config)?;
    }
    rewrite_gitmodules(&ws.root, &ws.config)?;
    Ok(())
}

fn ensure_odm_layout(root: &Path) -> Result<(), OdmError> {
    let odm = odm_dir(root);
    for name in ["cache", "log", "progen"] {
        let p = odm.join(name);
        fs::create_dir_all(&p).map_err(|e| {
            OdmError::operation(format!("failed to create {}: {e}", p.display()))
        })?;
    }
    Ok(())
}

fn collect_checks<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
) -> Result<Vec<DoctorCheck>, OdmError> {
    let mut checks = Vec::new();
    // Soft-load pin: invalid pin is reported by pin_checks, not a hard doctor error.
    let pin = load_pin(&ws.root).ok().flatten();
    let obs = observe_workspace(git, &ws.root, &ws.config, pin.as_ref())?;

    checks.push(DoctorCheck {
        id: "config_load".into(),
        status: CheckStatus::Pass,
        message: "config ok".into(),
        fixable: false,
    });

    checks.push(odm_layout_check(&ws.root));

    // path_declared + path_exists + managed_git + origin_match per entity
    for e in &obs.projects {
        push_entity_path_checks(&mut checks, &ws.root, "project", e);
    }
    for e in &obs.progens {
        push_entity_path_checks(&mut checks, &ws.root, "progen", e);
    }

    checks.push(gitignore_drift_check(ws, git)?);
    checks.extend(gitlink_checks(git, ws)?);
    checks.extend(pin_checks(ws)?);
    checks.extend(crate::doctor_worktree::worktree_checks(git, ws));

    Ok(checks)
}

fn odm_layout_check(root: &Path) -> DoctorCheck {
    let odm = odm_dir(root);
    let missing: Vec<&str> = ["cache", "log", "progen"]
        .into_iter()
        .filter(|n| !odm.join(n).is_dir())
        .collect();
    if missing.is_empty() {
        DoctorCheck {
            id: "odm_layout".into(),
            status: CheckStatus::Pass,
            message: ".odm cache/log/progen present".into(),
            fixable: true,
        }
    } else {
        DoctorCheck {
            id: "odm_layout".into(),
            status: CheckStatus::Warn,
            message: format!("missing .odm dirs: {}", missing.join(", ")),
            fixable: true,
        }
    }
}

fn push_entity_path_checks(
    checks: &mut Vec<DoctorCheck>,
    root: &Path,
    kind: &str,
    e: &EntityObservation,
) {
    let name = &e.name;
    let rel = &e.path;
    let id_base = format!("{kind}:{name}");

    if let Some(err) = &e.resolve_error {
        checks.push(DoctorCheck {
            id: format!("path_declared:{id_base}"),
            status: CheckStatus::Fail,
            message: err.clone(),
            fixable: false,
        });
        return;
    }

    checks.push(DoctorCheck {
        id: format!("path_declared:{id_base}"),
        status: CheckStatus::Pass,
        message: format!("{kind} '{name}' path under Workspace root"),
        fixable: false,
    });

    if !e.on_disk {
        checks.push(DoctorCheck {
            id: format!("path_exists:{id_base}"),
            status: CheckStatus::Warn,
            message: format!("{kind} '{name}' path missing on disk: {rel}"),
            fixable: false,
        });
        return;
    }

    checks.push(DoctorCheck {
        id: format!("path_exists:{id_base}"),
        status: CheckStatus::Pass,
        message: format!("{kind} '{name}' path exists"),
        fixable: false,
    });

    if !e.managed {
        return;
    }

    if !e.is_git {
        checks.push(DoctorCheck {
            id: format!("managed_git:{id_base}"),
            status: CheckStatus::Fail,
            message: format!("managed {kind} '{name}' exists but is not a git repo"),
            fixable: false,
        });
        return;
    }

    checks.push(DoctorCheck {
        id: format!("managed_git:{id_base}"),
        status: CheckStatus::Pass,
        message: format!("managed {kind} '{name}' is a git repo"),
        fixable: false,
    });

    let Some(cfg_url) = e.url.as_deref() else {
        return;
    };

    match &e.origin {
        Some(origin) => {
            if urls_match_with_root(cfg_url, origin, Some(root)) {
                checks.push(DoctorCheck {
                    id: format!("origin_match:{id_base}"),
                    status: CheckStatus::Pass,
                    message: format!("{kind} '{name}' origin matches config url"),
                    fixable: false,
                });
            } else {
                checks.push(DoctorCheck {
                    id: format!("origin_match:{id_base}"),
                    status: CheckStatus::Fail,
                    message: format!(
                        "{kind} '{name}' origin '{origin}' does not match config url '{cfg_url}'"
                    ),
                    fixable: false,
                });
            }
        }
        None => {
            checks.push(DoctorCheck {
                id: format!("origin_match:{id_base}"),
                status: CheckStatus::Fail,
                message: format!("{kind} '{name}' origin missing or unreadable"),
                fixable: false,
            });
        }
    }
}

fn gitlink_checks<R: odm_git::CommandRunner>(
    git: &Git<R>,
    ws: &Workspace,
) -> Result<Vec<DoctorCheck>, OdmError> {
    let mut checks = Vec::new();
    let drift = gitmodules_has_drift(&ws.root, &ws.config);
    checks.push(if drift {
        DoctorCheck {
            id: "gitmodules_layout".into(),
            status: CheckStatus::Fail,
            message: ".gitmodules differs from config gitlink entries".into(),
            fixable: true,
        }
    } else {
        DoctorCheck {
            id: "gitmodules_layout".into(),
            status: CheckStatus::Pass,
            message: ".gitmodules matches config gitlink entries".into(),
            fixable: true,
        }
    });

    let git_dir = ws.root.join(".git");
    if !git_dir.is_dir() && !git_dir.is_file() {
        return Ok(checks);
    }

    let managed = all_managed(&ws.config);
    let gitlink_paths: BTreeSet<String> = managed
        .iter()
        .filter(|e| e.checkout == CheckoutMode::Gitlink)
        .map(|e| e.path.replace('\\', "/"))
        .collect();
    let clone_paths: BTreeSet<String> = managed
        .iter()
        .filter(|e| e.checkout != CheckoutMode::Gitlink)
        .map(|e| e.path.replace('\\', "/"))
        .collect();
    let listed: BTreeSet<String> = git
        .list_gitlinks(&ws.root)?
        .into_iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();

    let extras: Vec<String> = listed
        .iter()
        .filter(|p| !gitlink_paths.contains(*p) && !clone_paths.contains(*p))
        .cloned()
        .collect();
    checks.push(named_list_check(
        "gitlink_extra",
        extras,
        "index gitlink not in config",
        "no extra gitlinks",
    ));

    let mut missing = Vec::new();
    let mut conflict = Vec::new();
    let mut mismatch = Vec::new();
    for e in &managed {
        let rec = git.gitlink_record(&ws.root, Path::new(&e.path))?;
        if e.checkout == CheckoutMode::Gitlink {
            match rec {
                GitlinkRecord::Missing => missing.push(e.name.clone()),
                GitlinkRecord::Conflict => conflict.push(e.name.clone()),
                GitlinkRecord::Recorded { .. } => {}
            }
            if let Ok(abs) = abs_checkout(&ws.root, &e.path) {
                if abs.join(".git").is_dir() {
                    mismatch.push(e.name.clone());
                }
            }
        } else if listed.contains(&e.path.replace('\\', "/")) {
            mismatch.push(e.name.clone());
        }
    }
    checks.push(named_list_check(
        "gitlink_missing",
        missing,
        "declared gitlink with no index record",
        "declared gitlinks have index records",
    ));
    checks.push(named_list_check(
        "gitlink_conflict",
        conflict,
        "gitlink index is in conflict",
        "no gitlink index conflicts",
    ));
    checks.push(named_list_check(
        "checkout_mismatch",
        mismatch,
        "checkout mode does not match occupancy",
        "checkout mode matches occupancy",
    ));
    Ok(checks)
}

fn named_list_check(id: &str, names: Vec<String>, fail_msg: &str, pass_msg: &str) -> DoctorCheck {
    if names.is_empty() {
        DoctorCheck {
            id: id.into(),
            status: CheckStatus::Pass,
            message: pass_msg.into(),
            fixable: false,
        }
    } else {
        DoctorCheck {
            id: id.into(),
            status: CheckStatus::Fail,
            message: format!("{fail_msg}: {}", names.join(", ")),
            fixable: false,
        }
    }
}

fn gitignore_drift_check<R: odm_git::CommandRunner>(
    ws: &Workspace,
    git: &Git<R>,
) -> Result<DoctorCheck, OdmError> {
    if !ws.config.manage_gitignore() {
        return Ok(DoctorCheck {
            id: "gitignore_drift".into(),
            status: CheckStatus::Pass,
            message: "manage_gitignore disabled".into(),
            fixable: false,
        });
    }
    let is_git = git.is_repo(&ws.root).unwrap_or(false);
    if !is_git {
        return Ok(DoctorCheck {
            id: "gitignore_drift".into(),
            status: CheckStatus::Pass,
            message: "Workspace is not a git repo".into(),
            fixable: false,
        });
    }
    let drift = workspace_gitignore_has_drift(&ws.root, &ws.config)?
        || ancestor_gitignore_has_drift(&ws.root, &ws.config)?;
    if drift {
        Ok(DoctorCheck {
            id: "gitignore_drift".into(),
            status: CheckStatus::Warn,
            message: "managed gitignore block differs from desired".into(),
            fixable: true,
        })
    } else {
        Ok(DoctorCheck {
            id: "gitignore_drift".into(),
            status: CheckStatus::Pass,
            message: "managed gitignore blocks match desired".into(),
            fixable: true,
        })
    }
}

fn pin_checks(ws: &Workspace) -> Result<Vec<DoctorCheck>, OdmError> {
    let mut checks = Vec::new();
    let path = pin_path(&ws.root);
    if !path.is_file() {
        checks.push(DoctorCheck {
            id: "pin_version".into(),
            status: CheckStatus::Pass,
            message: "no pin file".into(),
            fixable: false,
        });
        return Ok(checks);
    }

    let text = fs::read_to_string(&path).map_err(|e| {
        OdmError::workspace(format!("failed to read {}: {e}", path.display()))
    })?;

    // pin_version: invalid serde or version != 1 → fail
    let pin = match parse_pin_yaml(&text) {
        Ok(p) => {
            checks.push(DoctorCheck {
                id: "pin_version".into(),
                status: CheckStatus::Pass,
                message: "pin file version 1".into(),
                fixable: false,
            });
            p
        }
        Err(e) => {
            checks.push(DoctorCheck {
                id: "pin_version".into(),
                status: CheckStatus::Fail,
                message: e.message(),
                fixable: false,
            });
            // still try raw parse for rev format if possible
            return Ok(checks);
        }
    };

    let managed_names: std::collections::BTreeSet<String> = ws
        .config
        .projects
        .iter()
        .filter(|(_, e)| e.is_managed())
        .map(|(n, _)| n.clone())
        .chain(
            ws.config
                .progens
                .iter()
                .filter(|(_, e)| e.is_managed())
                .map(|(n, _)| n.clone()),
        )
        .collect();

    let unknown: Vec<_> = pin
        .pins
        .keys()
        .filter(|k| !managed_names.contains(k.as_str()))
        .cloned()
        .collect();
    if unknown.is_empty() {
        checks.push(DoctorCheck {
            id: "pin_unknown".into(),
            status: CheckStatus::Pass,
            message: "no unknown pin keys".into(),
            fixable: false,
        });
    } else {
        checks.push(DoctorCheck {
            id: "pin_unknown".into(),
            status: CheckStatus::Warn,
            message: format!("pin keys not in managed set: {}", unknown.join(", ")),
            fixable: false,
        });
    }

    // pin_rev_format — parse_pin_yaml already enforces; double-check for explicit check id
    let bad_revs: Vec<_> = pin
        .pins
        .iter()
        .filter(|(_, e)| !is_full_sha(&e.rev))
        .map(|(n, _)| n.clone())
        .collect();
    if bad_revs.is_empty() {
        checks.push(DoctorCheck {
            id: "pin_rev_format".into(),
            status: CheckStatus::Pass,
            message: "all pin revs are 40-char lowercase hex".into(),
            fixable: false,
        });
    } else {
        checks.push(DoctorCheck {
            id: "pin_rev_format".into(),
            status: CheckStatus::Fail,
            message: format!("invalid pin rev format: {}", bad_revs.join(", ")),
            fixable: false,
        });
    }

    // pin_url_mismatch
    let mut mismatches = Vec::new();
    for (name, pe) in &pin.pins {
        let cfg_url = ws
            .config
            .projects
            .get(name)
            .and_then(|e| e.url.as_deref())
            .or_else(|| {
                ws.config
                    .progens
                    .get(name)
                    .and_then(|e| e.url.as_deref())
            });
        if let Some(cu) = cfg_url {
            if !urls_match_with_root(cu, &pe.url, Some(&ws.root)) {
                mismatches.push(name.clone());
            }
        }
    }
    if mismatches.is_empty() {
        checks.push(DoctorCheck {
            id: "pin_url_mismatch".into(),
            status: CheckStatus::Pass,
            message: "pin urls match config".into(),
            fixable: false,
        });
    } else {
        checks.push(DoctorCheck {
            id: "pin_url_mismatch".into(),
            status: CheckStatus::Warn,
            message: format!("pin url mismatch for: {}", mismatches.join(", ")),
            fixable: false,
        });
    }

    Ok(checks)
}

/// Human multi-line doctor report.
pub fn format_doctor_human(report: &DoctorReport) -> String {
    let mut out = String::new();
    for c in &report.checks {
        let mark = match c.status {
            CheckStatus::Pass => "ok  ",
            CheckStatus::Warn => "warn",
            CheckStatus::Fail => "FAIL",
        };
        out.push_str(&format!("[{mark}] {id}: {msg}\n", id = c.id, msg = c.message));
    }
    if report.ok {
        out.push_str("doctor: ok\n");
    } else {
        out.push_str("doctor: failed\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    use crate::config::{ProjectEntry, WorkspaceConfig};
    use crate::init::{init_workspace, InitOptions};
    use crate::pin::{save_pin, PinEntry, PinFile};

    fn load_ws(root: &Path) -> Workspace {
        crate::config::load_workspace(root).unwrap()
    }

    #[test]
    fn config_load_always_pass() {
        let dir = tempdir().unwrap();
        init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: true,
            name: None,
        })
        .unwrap();
        let ws = load_ws(dir.path());
        let git = Git::new();
        let report = run_doctor(&git, &ws, false).unwrap();
        let c = report.checks.iter().find(|c| c.id == "config_load").unwrap();
        assert_eq!(c.status, CheckStatus::Pass);
        assert!(report.ok);
    }

    #[test]
    fn odm_layout_warn_and_fix() {
        let dir = tempdir().unwrap();
        init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: true,
            name: None,
        })
        .unwrap();
        let ws = load_ws(dir.path());
        let git = Git::new();
        let report = run_doctor(&git, &ws, false).unwrap();
        let c = report.checks.iter().find(|c| c.id == "odm_layout").unwrap();
        assert_eq!(c.status, CheckStatus::Warn);
        assert!(c.fixable);

        let report2 = run_doctor(&git, &ws, true).unwrap();
        let c2 = report2.checks.iter().find(|c| c.id == "odm_layout").unwrap();
        assert_eq!(c2.status, CheckStatus::Pass);
        assert!(dir.path().join(".odm/cache").is_dir());
        assert!(dir.path().join(".odm/log").is_dir());
        assert!(dir.path().join(".odm/progen").is_dir());
    }

    #[test]
    fn path_declared_escape_fails() {
        let dir = tempdir().unwrap();
        init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: true,
            name: None,
        })
        .unwrap();
        // Bypass load validation so doctor still samples escape at runtime
        // (load rejects `..`; observation/doctor share resolve_under_root).
        let mut cfg = WorkspaceConfig::default();
        cfg.projects.insert(
            "bad".into(),
            ProjectEntry {
                path: "../outside".into(),
                url: None,
                branch: None,
                type_: None,
                ..Default::default()
            },
        );
        let ws = Workspace {
            root: dir.path().to_path_buf(),
            config: cfg,
            actions: Default::default(),
            generators: Default::default(),
        };
        let git = Git::new();
        let report = run_doctor(&git, &ws, false).unwrap();
        let c = report
            .checks
            .iter()
            .find(|c| c.id.starts_with("path_declared:"))
            .unwrap();
        assert_eq!(c.status, CheckStatus::Fail);
        assert!(!report.ok);
    }

    #[test]
    fn gitignore_drift_fixable() {
        let dir = tempdir().unwrap();
        init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: false,
            name: None,
        })
        .unwrap();
        // clobber gitignore
        fs::write(dir.path().join(".gitignore"), "user\n").unwrap();
        let ws = load_ws(dir.path());
        let git = Git::new();
        let report = run_doctor(&git, &ws, false).unwrap();
        let c = report
            .checks
            .iter()
            .find(|c| c.id == "gitignore_drift")
            .unwrap();
        assert_eq!(c.status, CheckStatus::Warn);

        let report2 = run_doctor(&git, &ws, true).unwrap();
        let c2 = report2
            .checks
            .iter()
            .find(|c| c.id == "gitignore_drift")
            .unwrap();
        assert_eq!(c2.status, CheckStatus::Pass);
        let gi = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(gi.contains("user"));
        assert!(gi.contains(".odm/cache/"));
    }

    #[test]
    fn pin_unknown_warn() {
        let dir = tempdir().unwrap();
        init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: true,
            name: None,
        })
        .unwrap();
        let mut pin = PinFile::new_v1();
        pin.pins.insert(
            "ghost".into(),
            PinEntry {
                rev: "a".repeat(40),
                url: "https://example.com/x.git".into(),
                branch: None,
            },
        );
        save_pin(dir.path(), &pin).unwrap();
        let ws = load_ws(dir.path());
        let git = Git::new();
        let report = run_doctor(&git, &ws, false).unwrap();
        let c = report.checks.iter().find(|c| c.id == "pin_unknown").unwrap();
        assert_eq!(c.status, CheckStatus::Warn);
        assert!(report.ok); // warn only
    }

    #[test]
    fn pin_version_fail() {
        let dir = tempdir().unwrap();
        init_workspace(InitOptions {
            path: dir.path().to_path_buf(),
            no_git: true,
            name: None,
        })
        .unwrap();
        fs::write(
            dir.path().join(".odm/odm.lock.yaml"),
            "version: 99\npins: {}\n",
        )
        .unwrap();
        let ws = load_ws(dir.path());
        let git = Git::new();
        let report = run_doctor(&git, &ws, false).unwrap();
        let c = report.checks.iter().find(|c| c.id == "pin_version").unwrap();
        assert_eq!(c.status, CheckStatus::Fail);
        assert!(!report.ok);
    }

    #[test]
    fn check_status_classification() {
        assert_ne!(CheckStatus::Pass, CheckStatus::Fail);
        assert_ne!(CheckStatus::Warn, CheckStatus::Fail);
    }
}
