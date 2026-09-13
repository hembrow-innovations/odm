//! Store-oriented Progen façade: owns index lifecycle; no Connection at the boundary.

use std::path::PathBuf;

use odm_core::{OdmError, Workspace};
use rusqlite::Connection;
use serde::Serialize;

use crate::index::{
    self, get_indexed, load_all_notes, outgoing_links, resolve_link_target, search_fts, IndexStats,
    IndexedNote,
};
use crate::scope::{resolve_read_scope, resolve_single_progen, resolve_write_progen, ScopedProgen};

#[derive(Debug, Clone, Serialize)]
pub struct FindHit {
    pub progen: String,
    pub id: String,
    pub path: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LsHit {
    pub progen: String,
    pub id: String,
    pub path: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GetResult {
    pub progen: String,
    pub id: String,
    pub path: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextHit {
    pub progen: String,
    pub anchor: GetResult,
    pub outgoing: Vec<LsHit>,
    pub incoming: Vec<LsHit>,
}

/// Open handle to one Progen store (vault + disposable index).
pub struct ProgenStore {
    name: String,
    path: PathBuf,
    ws_root: PathBuf,
    conn: Connection,
}

impl ProgenStore {
    /// Ensure index exists and open it once for this handle's lifetime.
    pub fn open(ws: &Workspace, sp: &ScopedProgen) -> Result<Self, OdmError> {
        index::ensure_index(ws, sp)?;
        let conn = index::open_index(&ws.root, &sp.name)?;
        Ok(Self {
            name: sp.name.clone(),
            path: sp.path.clone(),
            ws_root: ws.root.clone(),
            conn,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn find(&self, query: &str, limit: usize) -> Result<Vec<FindHit>, OdmError> {
        let mut hits = Vec::new();
        for n in search_fts(&self.conn, query, limit)? {
            hits.push(FindHit {
                progen: self.name.clone(),
                id: n.id.clone(),
                path: n.rel_path.clone(),
                title: n.title.clone(),
                snippet: snippet(&n.body, query),
            });
        }
        Ok(hits)
    }

    pub fn list(&self) -> Result<Vec<LsHit>, OdmError> {
        Ok(load_all_notes(&self.conn)?
            .into_iter()
            .map(|n| self.to_ls(&n))
            .collect())
    }

    pub fn get(&self, id: &str) -> Result<GetResult, OdmError> {
        let n = get_indexed(&self.conn, id)?.ok_or_else(|| {
            OdmError::not_found(format!("no such note: {id} in progen '{}'", self.name))
        })?;
        Ok(self.to_get(&n))
    }

    /// In-store wikilink neighborhood (no cross-store walk).
    pub fn context(&self, id: &str) -> Result<ContextHit, OdmError> {
        let n = get_indexed(&self.conn, id)?.ok_or_else(|| {
            OdmError::not_found(format!("no such note: {id} in progen '{}'", self.name))
        })?;
        let anchor = self.to_get(&n);

        let mut outgoing = Vec::new();
        for t in outgoing_links(&self.conn, &n.id)? {
            if let Some(tn) = resolve_link_target(&self.conn, &t)? {
                outgoing.push(self.to_ls(&tn));
            } else {
                outgoing.push(LsHit {
                    progen: self.name.clone(),
                    id: t.clone(),
                    path: String::new(),
                    title: t,
                });
            }
        }

        let mut incoming_ids = std::collections::BTreeSet::new();
        for src in index::backlinks_for(&self.conn, &n.id)? {
            incoming_ids.insert(src);
        }
        incoming_ids.remove(&n.id);
        let mut incoming = Vec::new();
        for src in incoming_ids {
            if let Some(sn) = get_indexed(&self.conn, &src)? {
                incoming.push(self.to_ls(&sn));
            }
        }

        Ok(ContextHit {
            progen: self.name.clone(),
            anchor,
            outgoing,
            incoming,
        })
    }

    /// Sorted note paths (tree view).
    pub fn tree(&self) -> Result<Vec<String>, OdmError> {
        let mut paths: Vec<String> = self.list()?.into_iter().map(|h| h.path).collect();
        paths.sort();
        Ok(paths)
    }

    /// Notes that wikilink to id (incoming from context).
    pub fn backlinks(&self, id: &str) -> Result<Vec<LsHit>, OdmError> {
        Ok(self.context(id)?.incoming)
    }

    /// Rebuild index from vault; reconnects the handle.
    pub fn reindex(&mut self, ws: &Workspace) -> Result<IndexStats, OdmError> {
        let sp = ScopedProgen {
            name: self.name.clone(),
            path: self.path.clone(),
        };
        // Drop open DB handle before rebuild deletes the file.
        let _ = std::mem::replace(
            &mut self.conn,
            Connection::open_in_memory()
                .map_err(|e| OdmError::operation(format!("close index: {e}")))?,
        );
        let stats = index::reindex_progen(ws, &sp)?;
        self.conn = index::open_index(&self.ws_root, &self.name)?;
        Ok(stats)
    }

    fn to_ls(&self, n: &IndexedNote) -> LsHit {
        LsHit {
            progen: self.name.clone(),
            id: n.id.clone(),
            path: n.rel_path.clone(),
            title: n.title.clone(),
        }
    }

    fn to_get(&self, n: &IndexedNote) -> GetResult {
        GetResult {
            progen: self.name.clone(),
            id: n.id.clone(),
            path: n.rel_path.clone(),
            title: n.title.clone(),
            body: n.body.clone(),
        }
    }
}

/// Federated find across a resolved read scope.
pub fn find_notes(
    ws: &Workspace,
    query: &str,
    progens: &[String],
    groups: &[String],
    limit_per: usize,
) -> Result<Vec<FindHit>, OdmError> {
    let scope = resolve_read_scope(ws, progens, groups)?;
    let mut hits = Vec::new();
    for sp in &scope {
        let store = ProgenStore::open(ws, sp)?;
        hits.extend(store.find(query, limit_per)?);
    }
    Ok(hits)
}

/// List notes in one Progen (single-root).
pub fn list_notes(ws: &Workspace, progen: Option<&str>) -> Result<Vec<LsHit>, OdmError> {
    let sp = resolve_single_progen(ws, progen)?;
    ProgenStore::open(ws, &sp)?.list()
}

/// Get one note. Id may be `name:id` or bare id with optional --progen.
pub fn get_note(ws: &Workspace, id_arg: &str, progen: Option<&str>) -> Result<GetResult, OdmError> {
    let (sp, id) = resolve_id(ws, id_arg, progen)?;
    ProgenStore::open(ws, &sp)?.get(&id)
}

/// In-store wikilink neighborhood (no cross-store walk).
pub fn context_notes(
    ws: &Workspace,
    id_arg: &str,
    progen: Option<&str>,
) -> Result<ContextHit, OdmError> {
    let (sp, id) = resolve_id(ws, id_arg, progen)?;
    ProgenStore::open(ws, &sp)?.context(&id)
}

/// Open single-root store for tree/backlinks/get-style CLI ops.
pub fn open_single(ws: &Workspace, progen: Option<&str>) -> Result<ProgenStore, OdmError> {
    let sp = resolve_single_progen(ws, progen)?;
    ProgenStore::open(ws, &sp)
}

/// Open store for an id that may be `name:id` or bare with optional --progen.
pub fn open_for_id(
    ws: &Workspace,
    id_arg: &str,
    progen: Option<&str>,
) -> Result<(ProgenStore, String), OdmError> {
    let (sp, id) = resolve_id(ws, id_arg, progen)?;
    Ok((ProgenStore::open(ws, &sp)?, id))
}

/// Rebuild indexes for every Progen in `scope`.
pub fn reindex_scope(ws: &Workspace, scope: &[ScopedProgen]) -> Result<Vec<IndexStats>, OdmError> {
    let mut stats = Vec::with_capacity(scope.len());
    for sp in scope {
        stats.push(index::reindex_progen(ws, sp)?);
    }
    Ok(stats)
}

/// Reindex one Progen by optional flag (or full read scope when none).
pub fn reindex_for_cli(
    ws: &Workspace,
    progen_flags: &[String],
) -> Result<Vec<IndexStats>, OdmError> {
    let scope = if progen_flags.is_empty() {
        resolve_read_scope(ws, &[], &[])?
    } else if progen_flags.len() == 1 {
        vec![resolve_write_progen(ws, Some(progen_flags[0].as_str()))?]
    } else {
        return Err(OdmError::usage(
            "progen reindex: pass one --progen or none for all",
        ));
    };
    reindex_scope(ws, &scope)
}

/// At most one `--progen` flag; returns its value or `None`.
pub fn one_progen_flag(
    flags: &[String],
    usage_when_many: impl Into<String>,
) -> Result<Option<&str>, OdmError> {
    match flags {
        [] => Ok(None),
        [p] => Ok(Some(p.as_str())),
        _ => Err(OdmError::usage(usage_when_many)),
    }
}

fn resolve_id(
    ws: &Workspace,
    id_arg: &str,
    progen: Option<&str>,
) -> Result<(ScopedProgen, String), OdmError> {
    if let Some((name, id)) = id_arg.split_once(':') {
        if ws.config.progens.contains_key(name) {
            if let Some(flag) = progen {
                if flag != name {
                    return Err(OdmError::usage(format!(
                        "id prefix progen '{name}' conflicts with --progen '{flag}'"
                    )));
                }
            }
            let sp = resolve_single_progen(ws, Some(name))?;
            return Ok((sp, id.to_string()));
        }
    }
    let sp = resolve_single_progen(ws, progen)?;
    Ok((sp, id_arg.to_string()))
}

fn snippet(body: &str, query: &str) -> Option<String> {
    if query.is_empty() {
        return None;
    }
    let lower = body.to_lowercase();
    let q = query.to_lowercase();
    let idx = lower.find(&q)?;
    let start = body.floor_char_boundary(idx.saturating_sub(40).min(body.len()));
    let end = body.ceil_char_boundary((idx + q.len() + 40).min(body.len()));
    let mut s = body[start..end].replace('\n', " ");
    if start > 0 {
        s.insert(0, '…');
    }
    if end < body.len() {
        s.push('…');
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::ensure_vault;
    use odm_core::{ProgenEntry, WorkspaceConfig};
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::tempdir;

    fn ws_one(root: &std::path::Path, vault_rel: &str, name: &str) -> (Workspace, ScopedProgen) {
        let vault = root.join(vault_rel);
        ensure_vault(&vault).unwrap();
        let mut progens = BTreeMap::new();
        progens.insert(
            name.into(),
            ProgenEntry {
                path: vault_rel.into(),
                url: None,
                branch: None,
                ..Default::default()
            },
        );
        fs::create_dir_all(root.join(".odm")).unwrap();
        let ws = Workspace {
            root: root.to_path_buf(),
            config: WorkspaceConfig {
                progens,
                ..Default::default()
            },
            actions: BTreeMap::new(),
            generators: BTreeMap::new(),
        };
        let sp = ScopedProgen {
            name: name.into(),
            path: vault,
        };
        (ws, sp)
    }

    #[test]
    fn open_find_get_without_caller_touching_index() {
        let d = tempdir().unwrap();
        let root = d.path();
        let (ws, sp) = ws_one(root, "mem", "main");
        fs::write(
            sp.path.join("alpha.md"),
            "---\nid: a1\ntitle: Alpha\n---\nUniqueZebra word and [[Beta]].\n",
        )
        .unwrap();
        fs::write(
            sp.path.join("beta.md"),
            "---\nid: b1\ntitle: Beta\n---\nOther.\n",
        )
        .unwrap();

        let store = ProgenStore::open(&ws, &sp).unwrap();
        let hits = store.find("UniqueZebra", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "a1");
        assert_eq!(hits[0].progen, "main");

        let g = store.get("a1").unwrap();
        assert!(g.body.contains("UniqueZebra"));

        let ctx = store.context("a1").unwrap();
        assert!(ctx
            .outgoing
            .iter()
            .any(|h| h.id == "b1" || h.title == "Beta"));
        assert!(store.backlinks("b1").unwrap().iter().any(|h| h.id == "a1"));

        let paths = store.tree().unwrap();
        assert!(paths.iter().any(|p| p == "alpha.md"));
    }

    #[test]
    fn context_resolves_nested_basename_links() {
        let d = tempdir().unwrap();
        let root = d.path();
        let (ws, sp) = ws_one(root, "mem", "main");
        fs::create_dir_all(sp.path.join("sub")).unwrap();
        fs::write(
            sp.path.join("sub/alpha.md"),
            "---\nid: a1\ntitle: Alpha\n---\nSee [[note-beta|Beta alias]].\n",
        )
        .unwrap();
        fs::write(
            sp.path.join("sub/note-beta.md"),
            "---\nid: b1\ntitle: Note Beta\n---\nOther.\n",
        )
        .unwrap();

        let store = ProgenStore::open(&ws, &sp).unwrap();
        let ctx = store.context("a1").unwrap();
        assert_eq!(ctx.outgoing.len(), 1);
        assert_eq!(ctx.outgoing[0].id, "b1");
        assert_eq!(ctx.outgoing[0].path, "sub/note-beta.md");

        let ctx = store.context("b1").unwrap();
        assert!(ctx.incoming.iter().any(|h| h.id == "a1"));
        assert!(store.backlinks("b1").unwrap().iter().any(|h| h.id == "a1"));
        assert!(store
            .backlinks("note-beta")
            .unwrap()
            .iter()
            .any(|h| h.id == "a1"));
    }

    #[test]
    fn reindex_on_handle_and_scope() {
        let d = tempdir().unwrap();
        let root = d.path();
        let (ws, sp) = ws_one(root, "mem", "main");
        fs::write(sp.path.join("n.md"), "---\nid: n1\n---\nhello\n").unwrap();

        let mut store = ProgenStore::open(&ws, &sp).unwrap();
        assert_eq!(store.list().unwrap().len(), 2); // README + n

        fs::write(sp.path.join("m.md"), "---\nid: m1\n---\nmore\n").unwrap();
        let stats = store.reindex(&ws).unwrap();
        assert!(stats.notes >= 3);
        assert_eq!(store.get("m1").unwrap().id, "m1");

        let scope_stats = reindex_scope(&ws, &[sp]).unwrap();
        assert_eq!(scope_stats.len(), 1);
        assert_eq!(scope_stats[0].progen, "main");
    }

    #[test]
    fn federated_find_merges_stores() {
        let d = tempdir().unwrap();
        let root = d.path();
        let va = root.join("va");
        let vb = root.join("vb");
        ensure_vault(&va).unwrap();
        ensure_vault(&vb).unwrap();
        fs::write(
            va.join("a.md"),
            "---\nid: onlya\n---\nsharedtoken alphaonly\n",
        )
        .unwrap();
        fs::write(
            vb.join("b.md"),
            "---\nid: onlyb\n---\nsharedtoken betaonly\n",
        )
        .unwrap();

        let mut progens = BTreeMap::new();
        progens.insert(
            "a".into(),
            ProgenEntry {
                path: "va".into(),
                url: None,
                branch: None,
                ..Default::default()
            },
        );
        progens.insert(
            "b".into(),
            ProgenEntry {
                path: "vb".into(),
                url: None,
                branch: None,
                ..Default::default()
            },
        );
        fs::create_dir_all(root.join(".odm")).unwrap();
        let ws = Workspace {
            root: root.to_path_buf(),
            config: WorkspaceConfig {
                progens,
                ..Default::default()
            },
            actions: BTreeMap::new(),
            generators: BTreeMap::new(),
        };

        let hits = find_notes(&ws, "sharedtoken", &[], &[], 200).unwrap();
        assert_eq!(hits.len(), 2);

        let only_a = find_notes(&ws, "sharedtoken", &["a".into()], &[], 200).unwrap();
        assert_eq!(only_a.len(), 1);
        assert_eq!(only_a[0].progen, "a");
    }

    #[test]
    fn one_progen_flag_cardinality() {
        assert!(one_progen_flag(&[], "x").unwrap().is_none());
        assert_eq!(one_progen_flag(&["a".into()], "x").unwrap(), Some("a"));
        assert!(one_progen_flag(&["a".into(), "b".into()], "too many").is_err());
    }

    fn ws_two(root: &std::path::Path) -> Workspace {
        for rel in ["va", "vb"] {
            ensure_vault(&root.join(rel)).unwrap();
        }
        let mut progens = BTreeMap::new();
        progens.insert(
            "a".into(),
            ProgenEntry {
                path: "va".into(),
                url: None,
                branch: None,
                ..Default::default()
            },
        );
        progens.insert(
            "b".into(),
            ProgenEntry {
                path: "vb".into(),
                url: None,
                branch: None,
                ..Default::default()
            },
        );
        fs::create_dir_all(root.join(".odm")).unwrap();
        Workspace {
            root: root.to_path_buf(),
            config: WorkspaceConfig {
                progens,
                ..Default::default()
            },
            actions: BTreeMap::new(),
            generators: BTreeMap::new(),
        }
    }

    #[test]
    fn resolve_id_name_id_conflicts_with_progen_flag() {
        let d = tempdir().unwrap();
        let ws = ws_two(d.path());
        let err = resolve_id(&ws, "a:note1", Some("b")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("conflicts"), "{msg}");
        assert!(msg.contains("'a'"), "{msg}");
        assert!(msg.contains("'b'"), "{msg}");
    }

    #[test]
    fn resolve_id_name_id_matches_progen_flag() {
        let d = tempdir().unwrap();
        let ws = ws_two(d.path());
        let (sp, id) = resolve_id(&ws, "a:note1", Some("a")).unwrap();
        assert_eq!(sp.name, "a");
        assert_eq!(id, "note1");
    }

    #[test]
    fn multi_progen_read_message_has_no_write() {
        let d = tempdir().unwrap();
        let ws = ws_two(d.path());
        let err = list_notes(&ws, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("requires --progen"), "{msg}");
        assert!(!msg.to_lowercase().contains("write"), "{msg}");
    }

    #[test]
    fn snippet_empty_query_is_none() {
        assert_eq!(snippet("hello world", ""), None);
    }

    #[test]
    fn snippet_cjk_prefix_ascii_match_no_panic() {
        // Many 3-byte CJK chars so start = idx - 40 lands mid-char without boundary floor.
        let prefix: String = "你".repeat(30);
        let body = format!("{prefix}needle more text after the match site");
        let s = snippet(&body, "needle").expect("match");
        assert!(s.contains("needle"));
        assert!(s.starts_with('…') || s.starts_with('你'));
    }

    #[test]
    fn snippet_emoji_around_match_no_panic() {
        let body = format!("{}target{}", "🔥".repeat(20), "✨".repeat(20));
        let s = snippet(&body, "target").expect("match");
        assert!(s.contains("target"));
    }
}
