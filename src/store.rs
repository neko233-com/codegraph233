use crate::model::{Ref, RefRow, Symbol, SymbolRow};
use rusqlite::{params, Connection, Transaction};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Bump when the physical schema changes; a mismatch makes `index` rebuild the DB.
pub const SCHEMA_VERSION: i64 = 3;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE IF NOT EXISTS files (
  id INTEGER PRIMARY KEY, root TEXT NOT NULL, path TEXT NOT NULL, lang TEXT NOT NULL,
  module TEXT, size INTEGER NOT NULL, mtime INTEGER NOT NULL, UNIQUE(root, path));
CREATE TABLE IF NOT EXISTS symbols (
  id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL, root TEXT NOT NULL, lang TEXT NOT NULL,
  kind TEXT NOT NULL, name TEXT NOT NULL, qualname TEXT NOT NULL, namespace TEXT,
  parent_id INTEGER DEFAULT 0, start_line INTEGER, end_line INTEGER, signature TEXT,
  modifiers TEXT, param_count INTEGER DEFAULT -1, container TEXT, module TEXT);
CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_id);
CREATE TABLE IF NOT EXISTS refs (
  id INTEGER PRIMARY KEY, src_id INTEGER NOT NULL DEFAULT 0,
  file_id INTEGER NOT NULL, name TEXT NOT NULL,
  receiver_type TEXT, kind TEXT NOT NULL, namespace_hint TEXT, line INTEGER,
  arity INTEGER DEFAULT -1, target INTEGER DEFAULT 0, confidence INTEGER DEFAULT 0);
CREATE INDEX IF NOT EXISTS idx_refs_file ON refs(file_id);
CREATE INDEX IF NOT EXISTS idx_refs_unresolved ON refs(id) WHERE COALESCE(target,0)=0;
CREATE TABLE IF NOT EXISTS edges (
  src INTEGER NOT NULL, dst INTEGER NOT NULL, kind TEXT NOT NULL, line INTEGER,
  confidence INTEGER DEFAULT 0);
CREATE INDEX IF NOT EXISTS idx_edges_src ON edges(src);
CREATE INDEX IF NOT EXISTS idx_edges_dst ON edges(dst);
CREATE TABLE IF NOT EXISTS assets (
  id INTEGER PRIMARY KEY, asset_key TEXT NOT NULL UNIQUE, root TEXT, kind TEXT NOT NULL,
  path TEXT, name TEXT, unity_id TEXT, line INTEGER, meta TEXT);
CREATE INDEX IF NOT EXISTS idx_assets_path ON assets(path);
CREATE INDEX IF NOT EXISTS idx_assets_kind ON assets(kind);
CREATE INDEX IF NOT EXISTS idx_assets_kind_path ON assets(kind, path);
CREATE TABLE IF NOT EXISTS asset_edges (
  src TEXT NOT NULL, dst TEXT NOT NULL, kind TEXT NOT NULL, line INTEGER, detail TEXT, root TEXT);
CREATE INDEX IF NOT EXISTS idx_asset_edges_src ON asset_edges(src);
CREATE INDEX IF NOT EXISTS idx_asset_edges_dst ON asset_edges(dst);
CREATE INDEX IF NOT EXISTS idx_asset_edges_kind ON asset_edges(kind);
CREATE TABLE IF NOT EXISTS guid_map (guid TEXT PRIMARY KEY, path TEXT NOT NULL, name TEXT, type TEXT);
"#;

pub struct Store {
    pub conn: Connection,
}

pub struct FileState {
    pub id: i64,
    pub size: i64,
    pub mtime: i64,
    pub lang: String,
}

impl Store {
    pub fn open(path: &Path, fast: bool) -> Result<Store, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        if fast {
            conn.execute_batch(
                "PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA temp_store=MEMORY;
                 PRAGMA cache_size=-131072; PRAGMA mmap_size=268435456; PRAGMA busy_timeout=8000;",
            )
            .map_err(|e| e.to_string())?;
        } else {
            conn.execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA temp_store=MEMORY;
                 PRAGMA cache_size=-32000; PRAGMA mmap_size=268435456; PRAGMA busy_timeout=8000;",
            )
            .map_err(|e| e.to_string())?;
        }
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        // The asset-edge dedup index cannot be part of SCHEMA: older indexes may
        // already contain duplicate rows, so clean those once before creating it.
        let has_uniq: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_asset_edges_uniq'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if has_uniq == 0 {
            let _ = conn.execute(
                "DELETE FROM asset_edges WHERE rowid NOT IN
                 (SELECT MIN(rowid) FROM asset_edges GROUP BY src, dst, kind)",
                [],
            );
            let _ = conn.execute(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_asset_edges_uniq ON asset_edges(src, dst, kind)",
                [],
            );
        }
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);
        if version != SCHEMA_VERSION {
            // Fresh databases start at 0; only stamp the version when the DB has
            // no rows yet. An old populated DB keeps its version so callers can
            // report it as stale instead of silently serving wrong columns.
            let rows: i64 = conn
                .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
                .unwrap_or(0);
            if rows == 0 {
                conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(Store { conn })
    }

    /// True when a pre-existing DB was written by an older schema.
    pub fn is_stale(&self) -> Result<bool, String> {
        let v: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(v != SCHEMA_VERSION)
    }

    /// ANALYZE + VACUUM. Reclaims pages freed by incremental re-indexing.
    pub fn compact(&self) -> Result<(), String> {
        self.conn.execute_batch("ANALYZE").map_err(|e| e.to_string())?;
        self.conn.execute_batch("VACUUM").map_err(|e| e.to_string())
    }

    pub fn file_states(&self) -> Result<HashMap<String, FileState>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, root, path, size, mtime, lang FROM files")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                let root: String = r.get(1)?;
                let path: String = r.get(2)?;
                Ok((
                    format!("{root}|{path}"),
                    FileState {
                        id: r.get(0)?,
                        size: r.get(3)?,
                        mtime: r.get(4)?,
                        lang: r.get(5)?,
                    },
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut out = HashMap::new();
        for row in rows {
            let (k, v) = row.map_err(|e| e.to_string())?;
            out.insert(k, v);
        }
        Ok(out)
    }

    pub fn upsert_file(
        tx: &Transaction,
        root: &str,
        path: &str,
        lang: &str,
        module: &str,
        size: i64,
        mtime: i64,
    ) -> Result<i64, String> {
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM files WHERE root=?1 AND path=?2",
                params![root, path],
                |r| r.get(0),
            )
            .ok();
        if let Some(id) = existing {
            tx.execute(
                "UPDATE files SET lang=?1, module=?2, size=?3, mtime=?4 WHERE id=?5",
                params![lang, module, size, mtime, id],
            )
            .map_err(|e| e.to_string())?;
            return Ok(id);
        }
        tx.execute(
            "INSERT INTO files(root, path, lang, module, size, mtime) VALUES(?1,?2,?3,?4,?5,?6)",
            params![root, path, lang, module, size, mtime],
        )
        .map_err(|e| e.to_string())?;
        Ok(tx.last_insert_rowid())
    }

    pub fn insert_symbols(
        tx: &Transaction,
        file_id: i64,
        root: &str,
        syms: &[Symbol],
    ) -> Result<Vec<i64>, String> {
        let mut stmt = tx
            .prepare(
                "INSERT INTO symbols
                 (file_id, root, lang, kind, name, qualname, namespace, parent_id, start_line,
                  end_line, signature, modifiers, param_count, container, module)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,0,?8,?9,?10,?11,?12,?13,?14)",
            )
            .map_err(|e| e.to_string())?;
        let mut ids = Vec::with_capacity(syms.len());
        for s in syms {
            // Signatures are display-only; keeping the first line bounded keeps
            // the DB small on generated code with huge parameter lists.
            let sig = trim_signature(&s.signature);
            stmt.execute(params![
                file_id,
                root,
                s.lang,
                s.kind,
                s.name,
                s.qualname,
                s.namespace,
                s.start_line,
                s.end_line,
                sig,
                s.modifiers,
                s.param_count,
                s.container,
                s.module
            ])
            .map_err(|e| e.to_string())?;
            ids.push(tx.last_insert_rowid());
        }
        Ok(ids)
    }

    /// Insert refs for one file. `src_keys` maps a ref source key to its symbol
    /// id and namespace so repeated namespace hints stay out of the DB.
    pub fn insert_refs(
        tx: &Transaction,
        file_id: i64,
        refs: &[Ref],
        src_keys: &HashMap<String, (i64, String)>,
    ) -> Result<usize, String> {
        let mut stmt = tx
            .prepare(
                "INSERT INTO refs
                 (src_id, file_id, name, receiver_type, kind, namespace_hint, line, arity)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            )
            .map_err(|e| e.to_string())?;
        let mut n = 0;
        for r in refs {
            let hit = src_keys.get(&r.src_key);
            let src_id = hit.map(|h| h.0).unwrap_or(0);
            let redundant_ns = match hit {
                Some((id, ns)) => *id != 0 && *ns == r.namespace_hint,
                None => false,
            };
            let ns = if redundant_ns { "" } else { r.namespace_hint.as_str() };
            stmt.execute(params![
                src_id,
                file_id,
                r.name,
                r.receiver_type,
                r.kind,
                ns,
                r.line,
                r.arity
            ])
            .map_err(|e| e.to_string())?;
            n += 1;
        }
        Ok(n)
    }

    pub fn delete_file(tx: &Transaction, id: i64) -> Result<(), String> {
        tx.execute(
            "DELETE FROM edges WHERE src IN (SELECT id FROM symbols WHERE file_id=?1)",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        for sql in [
            "DELETE FROM symbols WHERE file_id=?1",
            "DELETE FROM refs WHERE file_id=?1",
            "DELETE FROM files WHERE id=?1",
        ] {
            tx.execute(sql, params![id]).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn symbol_rows(&self) -> Result<Vec<SymbolRow>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, root, lang, kind, name, qualname, COALESCE(namespace,''),
                        COALESCE(container,''), COALESCE(param_count,-1) FROM symbols",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SymbolRow {
                    id: r.get(0)?,
                    root: r.get(1)?,
                    lang: r.get(2)?,
                    kind: r.get(3)?,
                    name: r.get(4)?,
                    qualname: r.get(5)?,
                    namespace: r.get(6)?,
                    container: r.get(7)?,
                    param_count: r.get(8)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn ref_rows(&self) -> Result<Vec<RefRow>, String> {
        self.query_ref_rows(None, None)
    }

    /// Refs coming from `file_ids` or pointing at `target_ids`; either list may
    /// be empty. Used by the incremental rebind path.
    pub fn ref_rows_for(
        &self,
        file_ids: &[i64],
        target_ids: &[i64],
    ) -> Result<Vec<RefRow>, String> {
        let mut out: Vec<RefRow> = Vec::new();
        let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for chunk in file_ids.chunks(300) {
            for r in self.query_ref_rows(Some(chunk), None)? {
                if seen.insert(r.id) {
                    out.push(r);
                }
            }
        }
        for chunk in target_ids.chunks(300) {
            for r in self.query_ref_rows(None, Some(chunk))? {
                if seen.insert(r.id) {
                    out.push(r);
                }
            }
        }
        Ok(out)
    }

    fn query_ref_rows(
        &self,
        file_ids: Option<&[i64]>,
        target_ids: Option<&[i64]>,
    ) -> Result<Vec<RefRow>, String> {
        let mut sql = String::from(
            "SELECT id, src_id, name, COALESCE(receiver_type,''), kind,
                    COALESCE(namespace_hint,''), COALESCE(line,0), COALESCE(arity,-1),
                    COALESCE(target,0) FROM refs",
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let mut preds: Vec<String> = Vec::new();
        for (col, ids) in [("file_id", file_ids), ("target", target_ids)] {
            let Some(ids) = ids else { continue };
            if ids.is_empty() {
                continue;
            }
            let placeholders: Vec<String> = ids
                .iter()
                .map(|id| {
                    args.push(Box::new(*id));
                    format!("?{}", args.len())
                })
                .collect();
            preds.push(format!("{col} IN ({})", placeholders.join(",")));
        }
        if !preds.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&preds.join(" OR "));
        }
        let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), |r| {
                Ok(RefRow {
                    id: r.get(0)?,
                    src_id: r.get(1)?,
                    name: r.get(2)?,
                    receiver_type: r.get(3)?,
                    kind: r.get(4)?,
                    namespace_hint: r.get(5)?,
                    line: r.get(6)?,
                    arity: r.get(7)?,
                    target: r.get(8)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn symbol_ids_of_file(conn: &Connection, file_id: i64) -> Result<Vec<i64>, String> {
        let mut stmt = conn
            .prepare("SELECT id FROM symbols WHERE file_id=?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![file_id], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// Previously unresolved refs whose name now exists in the index. This lets
    /// an incremental pass pick up edges created by newly added symbols.
    pub fn ref_rows_unresolved_named(&self, names: &[String]) -> Result<Vec<RefRow>, String> {
        let mut out = Vec::new();
        for chunk in names.chunks(300) {
            if chunk.is_empty() {
                continue;
            }
            let list = chunk
                .iter()
                .map(|n| format!("'{}'", n.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT id, src_id, name, COALESCE(receiver_type,''), kind,
                        COALESCE(namespace_hint,''), COALESCE(line,0), COALESCE(arity,-1),
                        COALESCE(target,0)
                 FROM refs WHERE target=0 AND name IN ({list})"
            );
            let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(RefRow {
                        id: r.get(0)?,
                        src_id: r.get(1)?,
                        name: r.get(2)?,
                        receiver_type: r.get(3)?,
                        kind: r.get(4)?,
                        namespace_hint: r.get(5)?,
                        line: r.get(6)?,
                        arity: r.get(7)?,
                        target: r.get(8)?,
                    })
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                out.push(row.map_err(|e| e.to_string())?);
            }
        }
        Ok(out)
    }

    /// Delete every edge whose source or destination is in `ids`.
    pub fn delete_edges_touching(
        tx: &Transaction,
        ids: &[i64],
    ) -> Result<usize, String> {
        let mut n = 0usize;
        for chunk in ids.chunks(400) {
            let list = chunk
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(",");
            n += tx
                .execute(
                    &format!("DELETE FROM edges WHERE src IN ({list}) OR dst IN ({list})"),
                    [],
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(n)
    }

    pub fn clear_edges(tx: &Transaction) -> Result<(), String> {
        tx.execute("DELETE FROM edges", [])
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn update_targets(
        tx: &Transaction,
        updates: &[(i64, i64, String)],
    ) -> Result<(), String> {
        if updates.is_empty() {
            return Ok(());
        }
        let mut stmt = tx
            .prepare("UPDATE refs SET target=?1, confidence=?2 WHERE id=?3")
            .map_err(|e| e.to_string())?;
        for (id, target, conf) in updates {
            stmt.execute(params![target, conf_code(conf), id])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn insert_edges(
        tx: &Transaction,
        edges: &[(i64, i64, String, i32, String)],
    ) -> Result<(), String> {
        if edges.is_empty() {
            return Ok(());
        }
        let mut stmt = tx
            .prepare("INSERT INTO edges(src, dst, kind, line, confidence) VALUES(?1,?2,?3,?4,?5)")
            .map_err(|e| e.to_string())?;
        for (s, d, k, l, c) in edges {
            stmt.execute(params![s, d, k, l, conf_code(c)])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn insert_assets(tx: &Transaction, assets: &[crate::model::Asset]) -> Result<(), String> {
        if assets.is_empty() {
            return Ok(());
        }
        // drop stale rows of the same path once per batch
        let mut paths: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for a in assets {
            if !a.path.is_empty() {
                paths.insert(a.path.as_str());
            }
        }
        for p in paths {
            // `guid_ref` rows share the target's path, so they must not take
            // part in path-scoped deletion: re-parsing an asset's `.meta`
            // would otherwise wipe every reference to that asset.
            tx.execute(
                "DELETE FROM asset_edges
                 WHERE src IN (SELECT asset_key FROM assets WHERE path=?1 AND kind<>'guid_ref')
                    OR dst IN (SELECT asset_key FROM assets WHERE path=?1 AND kind<>'guid_ref')",
                params![p],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "DELETE FROM assets WHERE path=?1 AND kind<>'guid_ref'",
                params![p],
            )
            .map_err(|e| e.to_string())?;
        }
        let mut stmt = tx
            .prepare(
                "INSERT INTO assets(asset_key, root, kind, path, name, unity_id, line, meta)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(asset_key) DO UPDATE SET root=excluded.root,
                   kind = CASE WHEN excluded.path <> '' THEN excluded.kind ELSE assets.kind END,
                   path = CASE WHEN excluded.path <> '' THEN excluded.path ELSE assets.path END,
                   name = CASE WHEN excluded.name <> '' THEN excluded.name ELSE assets.name END,
                   unity_id = COALESCE(NULLIF(excluded.unity_id,''), assets.unity_id),
                   meta=excluded.meta",
            )
            .map_err(|e| e.to_string())?;
        for a in assets {
            let meta = serde_json::to_string(&a.meta).unwrap_or_else(|_| "{}".into());
            stmt.execute(params![a.key, a.root, a.kind, a.path, a.name, a.unity_id, a.line, meta])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn insert_asset_edges(tx: &Transaction, edges: &[crate::model::AssetEdge]) -> Result<(), String> {
        if edges.is_empty() {
            return Ok(());
        }
        let mut stmt = tx
            .prepare("INSERT OR IGNORE INTO asset_edges(src, dst, kind, line, detail, root) VALUES(?1,?2,?3,?4,?5,?6)")
            .map_err(|e| e.to_string())?;
        for e in edges {
            stmt.execute(params![e.src, e.dst, e.kind, e.line, e.detail, e.root])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn replace_guid_map(&self, rows: &[crate::model::GuidRow]) -> Result<(), String> {
        if rows.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM guid_map", []).map_err(|e| e.to_string())?;
        let mut stmt = tx
            .prepare("INSERT OR IGNORE INTO guid_map(guid, path, name, type) VALUES(?1,?2,?3,?4)")
            .map_err(|e| e.to_string())?;
        for r in rows {
            let name = std::path::Path::new(&r.path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            stmt.execute(params![r.guid, r.path, name, r.kind]).map_err(|e| e.to_string())?;
        }
        drop(stmt);
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn materialize_guid_edges(&self) -> Result<usize, String> {
        let distinct_guids = |s: &Store| -> i64 {
            s.conn
                .query_row(
                    "SELECT COUNT(DISTINCT dst) FROM asset_edges WHERE dst LIKE 'guid:%'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0)
        };
        let before = distinct_guids(self);
        let n = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO asset_edges(src, dst, kind, line, detail, root)
                 SELECT e.src,
                        'asset:' || COALESCE((SELECT a.root FROM assets a WHERE a.asset_key=e.src), e.root, '') || ':' || g.path,
                        e.kind, e.line, e.detail, e.root
                 FROM asset_edges e JOIN guid_map g ON g.guid = SUBSTR(e.dst, 6)
                 WHERE e.dst LIKE 'guid:%'",
                [],
            )
            .map_err(|e| e.to_string())?;
        // Once a guid resolves, the raw `guid:` edge is redundant; dropping it
        // keeps asset_info/refs output readable and the DB smaller.
        self.conn
            .execute(
                "DELETE FROM asset_edges
                 WHERE dst LIKE 'guid:%' AND SUBSTR(dst, 6) IN (SELECT guid FROM guid_map)",
                [],
            )
            .map_err(|e| e.to_string())?;
        let remaining = distinct_guids(self);
        self.set_meta("guid_refs", &before.to_string())?;
        self.set_meta("guid_refs_mapped", &(before - remaining).to_string())?;
        Ok(n)
    }

    pub fn guid_count(&self) -> i64 {
        self.conn
            .query_row("SELECT COUNT(*) FROM guid_map", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0)
    }

    pub fn insert_assets_and_edges(
        tx: &Transaction,
        assets: &[crate::model::Asset],
        edges: &[crate::model::AssetEdge],
    ) -> Result<(), String> {
        Store::insert_assets(tx, assets)?;
        Store::insert_asset_edges(tx, edges)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO meta(key,value) VALUES(?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> String {
        self.conn
            .query_row("SELECT value FROM meta WHERE key=?1", params![key], |r| {
                r.get::<_, String>(0)
            })
            .unwrap_or_default()
    }

    pub fn counts(&self) -> (i64, i64, i64, i64) {
        let q = |sql: &str| -> i64 {
            self.conn
                .query_row(sql, [], |r| r.get::<_, i64>(0))
                .unwrap_or(0)
        };
        (
            q("SELECT COUNT(*) FROM files"),
            q("SELECT COUNT(*) FROM symbols"),
            q("SELECT COUNT(*) FROM refs"),
            q("SELECT COUNT(*) FROM edges"),
        )
    }
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Confidence is one of a handful of strings; store it as a small integer to
/// avoid repeating text in millions of rows.
pub fn conf_code(conf: &str) -> i64 {
    match conf {
        "exact" => 1,
        "type" => 2,
        "name" => 3,
        "name-multi" => 4,
        _ => 0,
    }
}

pub fn conf_text(code: i64) -> &'static str {
    match code {
        1 => "exact",
        2 => "type",
        3 => "name",
        4 => "name-multi",
        _ => "unresolved",
    }
}

fn trim_signature(sig: &str) -> String {
    const MAX: usize = 200;
    let mut out = sig;
    if let Some(i) = out.find('\n') {
        out = &out[..i];
    }
    if out.len() > MAX {
        let mut cut = MAX;
        while cut > 0 && !out.is_char_boundary(cut) {
            cut -= 1;
        }
        let mut s = out[..cut].to_string();
        s.push('…');
        return s;
    }
    out.to_string()
}
