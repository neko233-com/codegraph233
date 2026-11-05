use crate::model::{Ref, RefRow, Symbol, SymbolRow};
use rusqlite::{params, Connection, Transaction};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

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
CREATE INDEX IF NOT EXISTS idx_symbols_qual ON symbols(qualname);
CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_id);
CREATE INDEX IF NOT EXISTS idx_symbols_container ON symbols(container);
CREATE TABLE IF NOT EXISTS refs (
  id INTEGER PRIMARY KEY, src_id INTEGER NOT NULL DEFAULT 0, src_key TEXT,
  file_id INTEGER NOT NULL, root TEXT, name TEXT NOT NULL, receiver TEXT,
  receiver_type TEXT, kind TEXT NOT NULL, namespace_hint TEXT, line INTEGER,
  arity INTEGER DEFAULT -1, target INTEGER DEFAULT 0, confidence TEXT);
CREATE INDEX IF NOT EXISTS idx_refs_target ON refs(target);
CREATE INDEX IF NOT EXISTS idx_refs_file ON refs(file_id);
CREATE INDEX IF NOT EXISTS idx_refs_name ON refs(name);
CREATE TABLE IF NOT EXISTS edges (
  src INTEGER NOT NULL, dst INTEGER NOT NULL, kind TEXT NOT NULL, line INTEGER, confidence TEXT);
CREATE INDEX IF NOT EXISTS idx_edges_src ON edges(src);
CREATE INDEX IF NOT EXISTS idx_edges_dst ON edges(dst);
CREATE TABLE IF NOT EXISTS assets (
  id INTEGER PRIMARY KEY, asset_key TEXT NOT NULL UNIQUE, root TEXT, kind TEXT NOT NULL,
  path TEXT, name TEXT, unity_id TEXT, line INTEGER, meta TEXT);
CREATE INDEX IF NOT EXISTS idx_assets_path ON assets(path);
CREATE INDEX IF NOT EXISTS idx_assets_kind ON assets(kind);
CREATE TABLE IF NOT EXISTS asset_edges (
  src TEXT NOT NULL, dst TEXT NOT NULL, kind TEXT NOT NULL, line INTEGER, detail TEXT, root TEXT);
CREATE INDEX IF NOT EXISTS idx_asset_edges_src ON asset_edges(src);
CREATE INDEX IF NOT EXISTS idx_asset_edges_dst ON asset_edges(dst);
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
                 PRAGMA cache_size=-32000; PRAGMA busy_timeout=8000;",
            )
            .map_err(|e| e.to_string())?;
        }
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        Ok(Store { conn })
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
            stmt.execute(params![
                file_id,
                s.root,
                s.lang,
                s.kind,
                s.name,
                s.qualname,
                s.namespace,
                s.start_line,
                s.end_line,
                s.signature,
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

    pub fn insert_refs(
        tx: &Transaction,
        file_id: i64,
        refs: &[Ref],
        key_to_id: &HashMap<String, i64>,
    ) -> Result<usize, String> {
        let mut stmt = tx
            .prepare(
                "INSERT INTO refs
                 (src_id, src_key, file_id, root, name, receiver, receiver_type, kind,
                  namespace_hint, line, arity)
                 VALUES(?1,?2,?3,?4,?5,'',?6,?7,?8,?9,?10)",
            )
            .map_err(|e| e.to_string())?;
        let mut n = 0;
        for r in refs {
            let src_id = key_to_id.get(&r.src_key).copied().unwrap_or(0);
            stmt.execute(params![
                src_id,
                r.src_key,
                file_id,
                r.root,
                r.name,
                r.receiver_type,
                r.kind,
                r.namespace_hint,
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
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, src_id, COALESCE(root,''), name, COALESCE(receiver_type,''), kind,
                        COALESCE(namespace_hint,''), COALESCE(line,0), COALESCE(arity,-1),
                        COALESCE(target,0) FROM refs",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(RefRow {
                    id: r.get(0)?,
                    src_id: r.get(1)?,
                    root: r.get(2)?,
                    name: r.get(3)?,
                    receiver_type: r.get(4)?,
                    kind: r.get(5)?,
                    namespace_hint: r.get(6)?,
                    line: r.get(7)?,
                    arity: r.get(8)?,
                    target: r.get(9)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
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
            stmt.execute(params![target, conf, id]).map_err(|e| e.to_string())?;
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
            stmt.execute(params![s, d, k, l, c]).map_err(|e| e.to_string())?;
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
            tx.execute(
                "DELETE FROM asset_edges WHERE src IN (SELECT asset_key FROM assets WHERE path=?1)
                 OR dst IN (SELECT asset_key FROM assets WHERE path=?1)",
                params![p],
            )
            .map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM assets WHERE path=?1", params![p])
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
