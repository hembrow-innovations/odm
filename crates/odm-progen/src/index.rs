use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use odm_core::{progen_index_dir, OdmError, Workspace};
use rusqlite::{params, Connection};

use crate::scope::ScopedProgen;
use crate::vault::walk_notes;

/// Fingerprint of vault note paths+mtimes (catches add/edit/delete).
const META_VAULT_FP: &str = "vault_fp";

/// Bump when the on-disk schema changes; `ensure_index` rebuilds stale indexes.
const SCHEMA_VERSION: &str = "2";

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS notes (
  id TEXT PRIMARY KEY,
  rel_path TEXT NOT NULL,
  title TEXT NOT NULL,
  body TEXT NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
  id UNINDEXED,
  title,
  body
);
CREATE TABLE IF NOT EXISTS links (
  src_id TEXT NOT NULL,
  target TEXT NOT NULL,
  target_id TEXT
);
"#;

#[derive(Debug, Clone)]
pub struct IndexStats {
    pub progen: String,
    pub notes: usize,
    pub links: usize,
}

/// `.odm/progen/<name>/`
pub(crate) fn index_dir(ws_root: &Path, progen_name: &str) -> PathBuf {
    progen_index_dir(ws_root, progen_name)
}

fn index_db_path(ws_root: &Path, progen_name: &str) -> PathBuf {
    index_dir(ws_root, progen_name).join("index.db")
}

/// Cheap paths+mtimes watermark; same skip rules as `walk_notes`.
fn vault_fingerprint(vault: &Path) -> Result<String, OdmError> {
    if !vault.is_dir() {
        return Ok(String::new());
    }
    let mut parts: Vec<String> = Vec::new();
    collect_fp(vault, vault, &mut parts)?;
    parts.sort();
    Ok(parts.join("\n"))
}

fn collect_fp(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), OdmError> {
    let entries = fs::read_dir(dir)
        .map_err(|e| OdmError::operation(format!("read {}: {e}", dir.display())))?;
    for ent in entries {
        let ent = ent.map_err(|e| OdmError::operation(e.to_string()))?;
        let path = ent.path();
        let name = ent.file_name();
        let name_s = name.to_string_lossy();
        if name_s.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_fp(root, &path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let meta = fs::metadata(&path)
                .map_err(|e| OdmError::operation(format!("stat {}: {e}", path.display())))?;
            let nanos = meta
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            out.push(format!("{rel}:{nanos}"));
        }
    }
    Ok(())
}

fn read_meta(conn: &Connection, key: &str) -> Result<Option<String>, OdmError> {
    let mut stmt = conn
        .prepare("SELECT value FROM meta WHERE key = ?1")
        .map_err(|e| OdmError::operation(e.to_string()))?;
    let mut rows = stmt
        .query_map(params![key], |row| row.get(0))
        .map_err(|e| OdmError::operation(e.to_string()))?;
    match rows.next() {
        Some(r) => Ok(Some(r.map_err(|e| OdmError::operation(e.to_string()))?)),
        None => Ok(None),
    }
}

fn index_stale(conn: &Connection, vault: &Path) -> Result<bool, OdmError> {
    let stored = match read_meta(conn, META_VAULT_FP)? {
        Some(s) => s,
        None => return Ok(true),
    };
    Ok(vault_fingerprint(vault)? != stored)
}

fn schema_current(conn: &Connection) -> Result<bool, OdmError> {
    Ok(read_meta(conn, "schema")?.as_deref() == Some(SCHEMA_VERSION))
}

/// Lightweight view of a note for link-target resolution.
struct NoteRef<'a> {
    id: &'a str,
    rel_path: &'a str,
    title: Option<&'a str>,
}

/// Resolve Obsidian-style wikilink targets to canonical note ids.
///
/// Candidate keys per note, best-first: exact id, case-insensitive id, basename
/// stem, rel_path without `.md`, rel_path with `.md`, case-insensitive title.
/// Keys claimed by more than one note at the same priority are ambiguous and
/// dropped (resolution stays conservative instead of guessing).
fn build_resolver<'a>(notes: impl Iterator<Item = NoteRef<'a>>) -> HashMap<String, String> {
    let mut map: HashMap<String, (u8, Option<String>)> = HashMap::new();
    for n in notes {
        let stem = Path::new(n.rel_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let no_ext = n.rel_path.strip_suffix(".md").unwrap_or(n.rel_path);
        let title = n.title.unwrap_or("").to_lowercase();
        let candidates = [
            (0, n.id.to_string()),
            (1, n.id.to_lowercase()),
            (2, stem.to_lowercase()),
            (3, no_ext.to_lowercase()),
            (4, n.rel_path.to_lowercase()),
            (5, title),
        ];
        for (prio, key) in candidates {
            if key.is_empty() {
                continue;
            }
            match map.get(&key) {
                None => {
                    map.insert(key, (prio, Some(n.id.to_string())));
                }
                Some(&(best, _)) if prio < best => {
                    map.insert(key, (prio, Some(n.id.to_string())));
                }
                Some(&(best, _)) if prio == best => {
                    map.insert(key, (best, None));
                }
                Some(_) => {}
            }
        }
    }
    map.into_iter()
        .filter_map(|(key, (_, id))| id.map(|id| (key, id)))
        .collect()
}

/// Rebuild disposable index for one Progen from vault files.
pub(crate) fn reindex_progen(ws: &Workspace, sp: &ScopedProgen) -> Result<IndexStats, OdmError> {
    let notes = walk_notes(&sp.path)?;
    let mut first_path: HashMap<&str, &str> = HashMap::new();
    for n in &notes {
        let id = n.id.as_str();
        if let Some(prev) = first_path.get(id) {
            return Err(OdmError::operation(format!(
                "duplicate note id '{id}': {prev} and {}",
                n.rel_path
            )));
        }
        first_path.insert(id, n.rel_path.as_str());
    }
    let resolver = build_resolver(notes.iter().map(|n| NoteRef {
        id: n.id.as_str(),
        rel_path: n.rel_path.as_str(),
        title: n.title.as_deref(),
    }));

    let watermark = vault_fingerprint(&sp.path)?;
    let dir = index_dir(&ws.root, &sp.name);
    fs::create_dir_all(&dir)
        .map_err(|e| OdmError::operation(format!("create index dir {}: {e}", dir.display())))?;
    let db = index_db_path(&ws.root, &sp.name);
    if db.exists() {
        let _ = fs::remove_file(&db);
    }

    let conn =
        Connection::open(&db).map_err(|e| OdmError::operation(format!("open index: {e}")))?;
    conn.execute_batch(SCHEMA)
        .map_err(|e| OdmError::operation(format!("index schema: {e}")))?;

    let mut link_count = 0usize;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| OdmError::operation(e.to_string()))?;
    {
        let mut ins = tx
            .prepare("INSERT INTO notes (id, rel_path, title, body) VALUES (?1, ?2, ?3, ?4)")
            .map_err(|e| OdmError::operation(e.to_string()))?;
        let mut ins_fts = tx
            .prepare("INSERT INTO notes_fts (id, title, body) VALUES (?1, ?2, ?3)")
            .map_err(|e| OdmError::operation(e.to_string()))?;
        let mut ins_link = tx
            .prepare("INSERT INTO links (src_id, target, target_id) VALUES (?1, ?2, ?3)")
            .map_err(|e| OdmError::operation(e.to_string()))?;

        for n in &notes {
            let title = n.title.as_deref().unwrap_or("");
            ins.execute(params![n.id.as_str(), n.rel_path, title, n.body])
                .map_err(|e| OdmError::operation(format!("index insert: {e}")))?;
            ins_fts
                .execute(params![n.id.as_str(), title, n.body])
                .map_err(|e| OdmError::operation(format!("fts insert: {e}")))?;
            for t in &n.wikilinks {
                let target_id = resolver.get(&t.to_lowercase()).cloned();
                ins_link
                    .execute(params![n.id.as_str(), t, target_id])
                    .map_err(|e| OdmError::operation(e.to_string()))?;
                link_count += 1;
            }
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('schema', ?1)",
        [SCHEMA_VERSION],
    )
    .map_err(|e| OdmError::operation(e.to_string()))?;
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![META_VAULT_FP, watermark],
    )
    .map_err(|e| OdmError::operation(e.to_string()))?;
    tx.commit()
        .map_err(|e| OdmError::operation(e.to_string()))?;

    Ok(IndexStats {
        progen: sp.name.clone(),
        notes: notes.len(),
        links: link_count,
    })
}

pub(crate) fn open_index(ws_root: &Path, progen_name: &str) -> Result<Connection, OdmError> {
    let db = index_db_path(ws_root, progen_name);
    if !db.exists() {
        return Err(OdmError::operation(format!(
            "index missing for progen '{progen_name}' (run `odm progen reindex`)"
        )));
    }
    Connection::open(&db).map_err(|e| OdmError::operation(format!("open index: {e}")))
}

/// Ensure index exists and is fresh vs vault mtimes; rebuild if missing or stale.
pub(crate) fn ensure_index(ws: &Workspace, sp: &ScopedProgen) -> Result<(), OdmError> {
    let db = index_db_path(&ws.root, &sp.name);
    if !db.exists() {
        reindex_progen(ws, sp)?;
        return Ok(());
    }
    let conn =
        Connection::open(&db).map_err(|e| OdmError::operation(format!("open index: {e}")))?;
    let stale = index_stale(&conn, &sp.path)?;
    let schema_stale = !schema_current(&conn)?;
    drop(conn);
    if stale || schema_stale {
        reindex_progen(ws, sp)?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct IndexedNote {
    pub id: String,
    pub rel_path: String,
    pub title: String,
    pub body: String,
}

pub(crate) fn load_all_notes(conn: &Connection) -> Result<Vec<IndexedNote>, OdmError> {
    let mut stmt = conn
        .prepare("SELECT id, rel_path, title, body FROM notes ORDER BY rel_path")
        .map_err(|e| OdmError::operation(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(IndexedNote {
                id: row.get(0)?,
                rel_path: row.get(1)?,
                title: row.get(2)?,
                body: row.get(3)?,
            })
        })
        .map_err(|e| OdmError::operation(e.to_string()))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| OdmError::operation(e.to_string()))?);
    }
    Ok(out)
}

/// Build an FTS5 MATCH expression from plain user text.
/// Each whitespace-separated token is double-quoted so operators (`AND`/`OR`/`NOT`)
/// and punctuation never parse as FTS syntax. Multi-word → implicit AND of terms.
/// Returns `None` when no usable tokens remain (caller treats as empty hits).
pub(crate) fn escape_fts_query(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.replace('\0', ""))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" "))
    }
}

fn is_fts_syntax_error(err: &rusqlite::Error) -> bool {
    let s = err.to_string();
    s.contains("fts5: syntax error") || s.contains("fts5: parse error")
}

pub(crate) fn search_fts(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<IndexedNote>, OdmError> {
    if query.trim().is_empty() {
        let mut all = load_all_notes(conn)?;
        all.truncate(limit);
        return Ok(all);
    }
    let Some(q) = escape_fts_query(query) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn
        .prepare("SELECT id, title, body FROM notes_fts WHERE notes_fts MATCH ?1 LIMIT ?2")
        .map_err(|e| OdmError::operation(e.to_string()))?;
    let mapped = stmt.query_map(params![q, limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    });
    let rows = match mapped {
        Ok(rows) => rows,
        Err(e) if is_fts_syntax_error(&e) => return Ok(Vec::new()),
        Err(e) => return Err(OdmError::operation(e.to_string())),
    };
    let mut fts_rows: Vec<(String, String, String)> = Vec::new();
    for r in rows {
        match r {
            Ok(row) => fts_rows.push(row),
            Err(e) if is_fts_syntax_error(&e) => return Ok(Vec::new()),
            Err(e) => return Err(OdmError::operation(e.to_string())),
        }
    }

    let mut out = Vec::new();
    for (id, _title, _body) in fts_rows {
        if let Some(n) = get_indexed(conn, &id)? {
            out.push(n);
        }
    }
    Ok(out)
}

pub(crate) fn get_indexed(conn: &Connection, id: &str) -> Result<Option<IndexedNote>, OdmError> {
    let mut stmt = conn
        .prepare("SELECT id, rel_path, title, body FROM notes WHERE id = ?1")
        .map_err(|e| OdmError::operation(e.to_string()))?;
    let mut rows = stmt
        .query_map(params![id], |row| {
            Ok(IndexedNote {
                id: row.get(0)?,
                rel_path: row.get(1)?,
                title: row.get(2)?,
                body: row.get(3)?,
            })
        })
        .map_err(|e| OdmError::operation(e.to_string()))?;
    if let Some(r) = rows.next() {
        return Ok(Some(r.map_err(|e| OdmError::operation(e.to_string()))?));
    }

    let md = if id.ends_with(".md") {
        id.to_string()
    } else {
        format!("{id}.md")
    };
    let basename_pattern = format!("%/{md}");
    let mut stmt = conn
        .prepare(
            "SELECT id, rel_path, title, body FROM notes
             WHERE rel_path = ?1 OR rel_path = ?2 OR rel_path LIKE ?3 OR title = ?4
             LIMIT 1",
        )
        .map_err(|e| OdmError::operation(e.to_string()))?;
    let mut rows = stmt
        .query_map(params![md.as_str(), id, basename_pattern, id], |row| {
            Ok(IndexedNote {
                id: row.get(0)?,
                rel_path: row.get(1)?,
                title: row.get(2)?,
                body: row.get(3)?,
            })
        })
        .map_err(|e| OdmError::operation(e.to_string()))?;
    match rows.next() {
        Some(r) => Ok(Some(r.map_err(|e| OdmError::operation(e.to_string()))?)),
        None => Ok(None),
    }
}

pub(crate) fn outgoing_links(conn: &Connection, src_id: &str) -> Result<Vec<String>, OdmError> {
    let mut stmt = conn
        .prepare("SELECT target FROM links WHERE src_id = ?1 ORDER BY target")
        .map_err(|e| OdmError::operation(e.to_string()))?;
    let rows = stmt
        .query_map(params![src_id], |row| row.get(0))
        .map_err(|e| OdmError::operation(e.to_string()))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| OdmError::operation(e.to_string()))?);
    }
    Ok(out)
}

pub(crate) fn resolve_link_target(
    conn: &Connection,
    target: &str,
) -> Result<Option<IndexedNote>, OdmError> {
    if let Some(n) = get_indexed(conn, target)? {
        return Ok(Some(n));
    }
    let notes = load_all_notes(conn)?;
    let resolver = build_resolver(notes.iter().map(|n| NoteRef {
        id: n.id.as_str(),
        rel_path: n.rel_path.as_str(),
        title: Some(n.title.as_str()),
    }));
    match resolver.get(&target.to_lowercase()) {
        Some(id) => get_indexed(conn, id),
        None => Ok(None),
    }
}

pub(crate) fn backlinks_for(conn: &Connection, target: &str) -> Result<Vec<String>, OdmError> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT src_id FROM links
             WHERE target_id = ?1 OR target = ?1
             ORDER BY src_id",
        )
        .map_err(|e| OdmError::operation(e.to_string()))?;
    let rows = stmt
        .query_map(params![target], |row| row.get(0))
        .map_err(|e| OdmError::operation(e.to_string()))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| OdmError::operation(e.to_string()))?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::ensure_vault;
    use odm_core::{ProgenEntry, WorkspaceConfig};
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    #[test]
    fn reindex_and_search() {
        let d = tempdir().unwrap();
        let root = d.path();
        let vault = root.join("mem");
        ensure_vault(&vault).unwrap();
        fs::write(
            vault.join("alpha.md"),
            "---\nid: a1\ntitle: Alpha\n---\nUniqueZebra word and [[Beta]].\n",
        )
        .unwrap();
        fs::write(
            vault.join("beta.md"),
            "---\nid: b1\ntitle: Beta\n---\nOther.\n",
        )
        .unwrap();

        let mut progens = BTreeMap::new();
        progens.insert(
            "main".into(),
            ProgenEntry {
                path: "mem".into(),
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
            name: "main".into(),
            path: vault,
        };
        let stats = reindex_progen(&ws, &sp).unwrap();
        assert_eq!(stats.notes, 3);
        let conn = open_index(root, "main").unwrap();
        let hits = search_fts(&conn, "UniqueZebra", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "a1");
    }

    fn fixture_conn() -> (tempfile::TempDir, Connection) {
        let d = tempdir().unwrap();
        let root = d.path();
        let vault = root.join("mem");
        ensure_vault(&vault).unwrap();
        fs::write(
            vault.join("alpha.md"),
            "---\nid: a1\ntitle: Alpha\n---\nUniqueZebra word and beta notes.\n",
        )
        .unwrap();
        fs::write(
            vault.join("beta.md"),
            "---\nid: b1\ntitle: Beta\n---\nOther content here.\n",
        )
        .unwrap();
        let mut progens = BTreeMap::new();
        progens.insert(
            "main".into(),
            ProgenEntry {
                path: "mem".into(),
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
            name: "main".into(),
            path: vault,
        };
        reindex_progen(&ws, &sp).unwrap();
        let conn = open_index(root, "main").unwrap();
        (d, conn)
    }

    #[test]
    fn escape_fts_quotes_operators() {
        assert_eq!(escape_fts_query("AND").as_deref(), Some("\"AND\""));
        assert_eq!(
            escape_fts_query("foo OR bar").as_deref(),
            Some("\"foo\" \"OR\" \"bar\"")
        );
        assert_eq!(
            escape_fts_query(r#"say "hi""#).as_deref(),
            Some(r#""say" """hi""""#)
        );
        assert_eq!(escape_fts_query("   ").as_deref(), None);
    }

    #[test]
    fn search_and_alone_is_not_syntax_error() {
        let (_d, conn) = fixture_conn();
        let hits = search_fts(&conn, "AND", 10).unwrap();
        // literal token AND may or may not appear in corpus; must not error
        assert!(hits.len() <= 10);
    }

    #[test]
    fn search_punctuation_is_not_syntax_error() {
        let (_d, conn) = fixture_conn();
        let hits = search_fts(&conn, "@@@", 10).expect("no fts syntax");
        assert!(hits.is_empty());
        let hits = search_fts(&conn, r#""broken" OR ("#, 10).expect("no fts syntax");
        assert!(hits.is_empty());
    }

    #[test]
    fn search_multi_word_and_of_terms() {
        let (_d, conn) = fixture_conn();
        let hits = search_fts(&conn, "UniqueZebra word", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "a1");
    }

    fn ws_sp(root: &Path, vault_rel: &str, name: &str) -> (Workspace, ScopedProgen) {
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
    fn ensure_rebuilds_when_vault_newer_than_index() {
        let d = tempdir().unwrap();
        let root = d.path();
        let (ws, sp) = ws_sp(root, "mem", "main");
        fs::write(
            sp.path.join("alpha.md"),
            "---\nid: a1\ntitle: Alpha\n---\nOriginalToken here.\n",
        )
        .unwrap();
        reindex_progen(&ws, &sp).unwrap();
        let conn = open_index(root, "main").unwrap();
        assert_eq!(search_fts(&conn, "OriginalToken", 10).unwrap().len(), 1);
        assert!(search_fts(&conn, "EditedToken", 10).unwrap().is_empty());
        drop(conn);

        // Bump mtime past the watermark (1s resolution on some FS).
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(
            sp.path.join("alpha.md"),
            "---\nid: a1\ntitle: Alpha\n---\nEditedToken here.\n",
        )
        .unwrap();

        ensure_index(&ws, &sp).unwrap();
        let conn = open_index(root, "main").unwrap();
        assert!(search_fts(&conn, "OriginalToken", 10).unwrap().is_empty());
        let hits = search_fts(&conn, "EditedToken", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "a1");
    }

    #[test]
    fn reindex_duplicate_id_lists_both_paths() {
        let d = tempdir().unwrap();
        let root = d.path();
        let (ws, sp) = ws_sp(root, "mem", "main");
        fs::write(
            sp.path.join("one.md"),
            "---\nid: same\ntitle: One\n---\nA.\n",
        )
        .unwrap();
        fs::write(
            sp.path.join("two.md"),
            "---\nid: same\ntitle: Two\n---\nB.\n",
        )
        .unwrap();

        let err = reindex_progen(&ws, &sp).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate") && msg.contains("same"),
            "expected duplicate id error, got: {msg}"
        );
        assert!(
            msg.contains("one.md") && msg.contains("two.md"),
            "expected both paths, got: {msg}"
        );
    }

    #[test]
    fn reindex_invalid_frontmatter_errors_with_path() {
        let d = tempdir().unwrap();
        let root = d.path();
        let (ws, sp) = ws_sp(root, "mem", "main");
        fs::write(
            sp.path.join("broken.md"),
            "---\nid: [not closed\n---\nBody.\n",
        )
        .unwrap();

        let err = reindex_progen(&ws, &sp).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("broken.md") && msg.to_lowercase().contains("frontmatter"),
            "expected path + frontmatter error, got: {msg}"
        );
    }

    #[test]
    fn reindex_skips_wikilinks_in_fences() {
        let d = tempdir().unwrap();
        let root = d.path();
        let (ws, sp) = ws_sp(root, "mem", "main");
        fs::write(
            sp.path.join("n.md"),
            "---\nid: n1\n---\n[[Real]]\n```\n[[Fake]]\n```\n",
        )
        .unwrap();
        reindex_progen(&ws, &sp).unwrap();
        let conn = open_index(root, "main").unwrap();
        let links = outgoing_links(&conn, "n1").unwrap();
        assert_eq!(links, vec!["Real".to_string()]);
    }

    #[test]
    fn reindex_resolves_wikilink_targets_to_ids() {
        let d = tempdir().unwrap();
        let root = d.path();
        let (ws, sp) = ws_sp(root, "mem", "main");
        fs::create_dir_all(sp.path.join("sub")).unwrap();
        fs::write(
            sp.path.join("alpha.md"),
            "---\nid: a1\ntitle: Alpha\n---\nSee [[note-beta]], [[sub/note-beta|alias]], [[Note Beta]] and [[missing]].\n",
        )
        .unwrap();
        fs::write(
            sp.path.join("sub/note-beta.md"),
            "---\nid: b9\ntitle: Note Beta\n---\nOther.\n",
        )
        .unwrap();
        reindex_progen(&ws, &sp).unwrap();
        let conn = open_index(root, "main").unwrap();

        let back = backlinks_for(&conn, "b9").unwrap();
        assert_eq!(back, vec!["a1".to_string()]);

        let resolved = resolve_link_target(&conn, "sub/note-beta")
            .unwrap()
            .unwrap();
        assert_eq!(resolved.id, "b9");
        let resolved = resolve_link_target(&conn, "Note Beta").unwrap().unwrap();
        assert_eq!(resolved.id, "b9");
        assert!(resolve_link_target(&conn, "missing").unwrap().is_none());
    }

    #[test]
    fn backlinks_match_resolved_id_or_raw_target() {
        let d = tempdir().unwrap();
        let root = d.path();
        let (ws, sp) = ws_sp(root, "mem", "main");
        fs::write(
            sp.path.join("one.md"),
            "---\nid: o1\n---\nLink by id: [[b1]].\n",
        )
        .unwrap();
        fs::write(
            sp.path.join("two.md"),
            "---\nid: o2\n---\nLink by title: [[Beta]].\n",
        )
        .unwrap();
        fs::write(
            sp.path.join("beta.md"),
            "---\nid: b1\ntitle: Beta\n---\nBody.\n",
        )
        .unwrap();
        reindex_progen(&ws, &sp).unwrap();
        let conn = open_index(root, "main").unwrap();
        let back = backlinks_for(&conn, "b1").unwrap();
        assert_eq!(back, vec!["o1".to_string(), "o2".to_string()]);
    }

    #[test]
    fn ambiguous_targets_stay_unresolved() {
        let d = tempdir().unwrap();
        let root = d.path();
        let (ws, sp) = ws_sp(root, "mem", "main");
        fs::write(sp.path.join("one.md"), "---\nid: o1\n---\nLink: [[dup]]\n").unwrap();
        fs::create_dir_all(sp.path.join("sub")).unwrap();
        fs::write(sp.path.join("sub/dup.md"), "---\nid: d1\n---\nA.\n").unwrap();
        fs::create_dir_all(sp.path.join("other")).unwrap();
        fs::write(sp.path.join("other/dup.md"), "---\nid: d2\n---\nB.\n").unwrap();
        reindex_progen(&ws, &sp).unwrap();
        let conn = open_index(root, "main").unwrap();
        let links: Vec<(String, Option<String>)> = {
            let mut stmt = conn.prepare("SELECT target, target_id FROM links").unwrap();
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap();
            rows.collect::<Result<_, _>>().unwrap()
        };
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, "dup");
        assert_eq!(links[0].1, None);
    }

    #[test]
    fn ensure_rebuilds_when_schema_stale() {
        let d = tempdir().unwrap();
        let root = d.path();
        let (ws, sp) = ws_sp(root, "mem", "main");
        fs::write(sp.path.join("n.md"), "---\nid: n1\n---\n[[Other]]\n").unwrap();
        reindex_progen(&ws, &sp).unwrap();
        let conn = open_index(root, "main").unwrap();
        assert_eq!(
            read_meta(&conn, "schema").unwrap().as_deref(),
            Some(SCHEMA_VERSION)
        );
        conn.execute("UPDATE meta SET value = '1' WHERE key = 'schema'", [])
            .unwrap();
        drop(conn);

        ensure_index(&ws, &sp).unwrap();
        let conn = open_index(root, "main").unwrap();
        assert_eq!(
            read_meta(&conn, "schema").unwrap().as_deref(),
            Some(SCHEMA_VERSION)
        );
        let has_col: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('links') WHERE name = 'target_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(has_col, 1);
    }

    #[test]
    fn ensure_rebuilds_after_note_deleted() {
        let d = tempdir().unwrap();
        let root = d.path();
        let (ws, sp) = ws_sp(root, "mem", "main");
        fs::write(sp.path.join("keep.md"), "---\nid: k1\n---\nKeepToken\n").unwrap();
        fs::write(sp.path.join("gone.md"), "---\nid: g1\n---\nGoneToken\n").unwrap();
        reindex_progen(&ws, &sp).unwrap();
        let conn = open_index(root, "main").unwrap();
        assert_eq!(search_fts(&conn, "GoneToken", 10).unwrap().len(), 1);
        drop(conn);

        fs::remove_file(sp.path.join("gone.md")).unwrap();
        ensure_index(&ws, &sp).unwrap();
        let conn = open_index(root, "main").unwrap();
        assert!(search_fts(&conn, "GoneToken", 10).unwrap().is_empty());
        assert_eq!(search_fts(&conn, "KeepToken", 10).unwrap().len(), 1);
    }
}
