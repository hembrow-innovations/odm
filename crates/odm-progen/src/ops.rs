use odm_core::{OdmError, Workspace};
use serde::Serialize;

use crate::scope::{resolve_read_scope, resolve_write_progen};
use crate::store::{ContextHit, FindHit, GetResult, LsHit};
use crate::vault::vault_info;

pub fn format_find_human(hits: &[FindHit]) -> String {
    if hits.is_empty() {
        return "(no matches)\n".into();
    }
    let mut out = String::new();
    for h in hits {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            h.progen, h.id, h.path, h.title
        ));
    }
    out
}

pub fn format_ls_human(hits: &[LsHit]) -> String {
    if hits.is_empty() {
        return "(no notes)\n".into();
    }
    let mut out = String::new();
    for h in hits {
        out.push_str(&format!("{}\t{}\t{}\n", h.id, h.path, h.title));
    }
    out
}

pub fn format_get_human(g: &GetResult) -> String {
    format!(
        "---\nprogen: {}\nid: {}\npath: {}\ntitle: {}\n---\n{}",
        g.progen, g.id, g.path, g.title, g.body
    )
}

pub fn format_context_human(c: &ContextHit) -> String {
    let mut out = String::new();
    out.push_str(&format!("# context {}\n\n", c.anchor.id));
    out.push_str(&format_get_human(&c.anchor));
    out.push_str("\n\n## outgoing\n");
    if c.outgoing.is_empty() {
        out.push_str("(none)\n");
    } else {
        for h in &c.outgoing {
            out.push_str(&format!("- {} ({})\n", h.id, h.title));
        }
    }
    out.push_str("\n## incoming\n");
    if c.incoming.is_empty() {
        out.push_str("(none)\n");
    } else {
        for h in &c.incoming {
            out.push_str(&format!("- {} ({})\n", h.id, h.title));
        }
    }
    out
}

/// Store-side doctor checks for one or all progens.
pub fn doctor_progens(
    ws: &Workspace,
    progen: Option<&str>,
) -> Result<Vec<ProgenDoctorCheck>, OdmError> {
    let scope = if let Some(n) = progen {
        vec![resolve_write_progen(ws, Some(n))?]
    } else if ws.config.progens.is_empty() {
        return Ok(vec![]);
    } else {
        resolve_read_scope(ws, &[], &[])?
    };
    let mut checks = Vec::new();
    for sp in scope {
        let info = vault_info(&sp)?;
        checks.push(ProgenDoctorCheck {
            progen: sp.name.clone(),
            id: "vault_path".into(),
            ok: info.on_disk,
            message: if info.on_disk {
                format!("vault present ({})", sp.path.display())
            } else {
                format!("vault missing: {}", sp.path.display())
            },
        });
        let idx = odm_core::progen_index_dir(&ws.root, &sp.name).join("index.db");
        let idx_ok = idx.is_file();
        checks.push(ProgenDoctorCheck {
            progen: sp.name.clone(),
            id: "index".into(),
            ok: idx_ok,
            message: if idx_ok {
                format!("index present ({} notes)", info.note_count)
            } else {
                "index missing (run `odm progen reindex`)".into()
            },
        });
    }
    Ok(checks)
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgenDoctorCheck {
    pub progen: String,
    pub id: String,
    pub ok: bool,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ContextHit, FindHit, GetResult, LsHit};
    use crate::vault::ensure_vault;
    use odm_core::{progen_index_dir, ProgenEntry, WorkspaceConfig};
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::tempdir;

    fn hit(progen: &str, id: &str, path: &str, title: &str) -> FindHit {
        FindHit {
            progen: progen.into(),
            id: id.into(),
            path: path.into(),
            title: title.into(),
            snippet: None,
        }
    }

    fn get(progen: &str, id: &str, path: &str, title: &str, body: &str) -> GetResult {
        GetResult {
            progen: progen.into(),
            id: id.into(),
            path: path.into(),
            title: title.into(),
            body: body.into(),
        }
    }

    fn ls(progen: &str, id: &str, path: &str, title: &str) -> LsHit {
        LsHit {
            progen: progen.into(),
            id: id.into(),
            path: path.into(),
            title: title.into(),
        }
    }

    fn ws_with(
        root: std::path::PathBuf,
        progens: BTreeMap<String, ProgenEntry>,
    ) -> Workspace {
        Workspace {
            root,
            config: WorkspaceConfig {
                progens,
                ..Default::default()
            },
            actions: BTreeMap::new(),
            generators: BTreeMap::new(),
        }
    }

    #[test]
    fn format_find_human_empty() {
        assert_eq!(format_find_human(&[]), "(no matches)\n");
    }

    #[test]
    fn format_find_human_rows() {
        let hits = vec![
            hit("main", "a1", "a.md", "Alpha"),
            hit("main", "b1", "b.md", "Beta"),
        ];
        assert_eq!(
            format_find_human(&hits),
            "main\ta1\ta.md\tAlpha\nmain\tb1\tb.md\tBeta\n"
        );
    }

    #[test]
    fn format_context_human_empty_edges() {
        let c = ContextHit {
            progen: "main".into(),
            anchor: get("main", "a1", "a.md", "A", "body\n"),
            outgoing: vec![],
            incoming: vec![],
        };
        let s = format_context_human(&c);
        assert!(s.starts_with("# context a1\n"));
        assert!(s.contains("progen: main"));
        assert!(s.contains("id: a1"));
        assert!(s.contains("body\n"));
        assert!(s.contains("## outgoing\n(none)\n"));
        assert!(s.contains("## incoming\n(none)\n"));
    }

    #[test]
    fn format_context_human_with_edges() {
        let c = ContextHit {
            progen: "main".into(),
            anchor: get("main", "a1", "a.md", "A", "x"),
            outgoing: vec![ls("main", "b1", "b.md", "B")],
            incoming: vec![ls("main", "c1", "c.md", "C")],
        };
        let s = format_context_human(&c);
        assert!(s.contains("## outgoing\n- b1 (B)\n"));
        assert!(s.contains("## incoming\n- c1 (C)\n"));
        assert!(!s.contains("(none)"));
    }

    #[test]
    fn doctor_empty_config() {
        let d = tempdir().unwrap();
        let ws = ws_with(d.path().to_path_buf(), BTreeMap::new());
        assert!(doctor_progens(&ws, None).unwrap().is_empty());
    }

    #[test]
    fn doctor_missing_vault() {
        let d = tempdir().unwrap();
        let root = d.path();
        let mut progens = BTreeMap::new();
        progens.insert(
            "gone".into(),
            ProgenEntry {
                path: "vaults/gone".into(),
                url: None,
                branch: None,
                ..Default::default()
            },
        );
        let ws = ws_with(root.to_path_buf(), progens);
        let checks = doctor_progens(&ws, Some("gone")).unwrap();
        let vault = checks.iter().find(|c| c.id == "vault_path").unwrap();
        assert!(!vault.ok);
        assert!(vault.message.contains("vault missing"));
        let idx = checks.iter().find(|c| c.id == "index").unwrap();
        assert!(!idx.ok);
        assert!(idx.message.contains("index missing"));
    }

    #[test]
    fn doctor_vault_and_index_present() {
        let d = tempdir().unwrap();
        let root = d.path();
        let vault = root.join("vaults/main");
        ensure_vault(&vault).unwrap();
        fs::write(vault.join("n.md"), "---\nid: n1\n---\nhi\n").unwrap();
        let idx_dir = progen_index_dir(root, "main");
        fs::create_dir_all(&idx_dir).unwrap();
        fs::write(idx_dir.join("index.db"), b"stub").unwrap();

        let mut progens = BTreeMap::new();
        progens.insert(
            "main".into(),
            ProgenEntry {
                path: "vaults/main".into(),
                url: None,
                branch: None,
                ..Default::default()
            },
        );
        let ws = ws_with(root.to_path_buf(), progens);
        let checks = doctor_progens(&ws, None).unwrap();
        let vault_c = checks.iter().find(|c| c.id == "vault_path").unwrap();
        assert!(vault_c.ok);
        assert!(vault_c.message.contains("vault present"));
        let idx = checks.iter().find(|c| c.id == "index").unwrap();
        assert!(idx.ok);
        assert!(idx.message.contains("index present"));
        assert!(idx.message.contains("notes"));
    }
}
