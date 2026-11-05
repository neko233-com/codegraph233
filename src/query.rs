use crate::config::Config;
use crate::store::Store;
use rusqlite::params;
use serde_json::{json, Value};
use std::path::Path;

pub struct Engine {
    pub store: Store,
    pub cfg: Config,
}

impl Engine {
    pub fn open(cfg: &Config) -> Result<Engine, String> {
        let store = Store::open(&cfg.db_path(), false)?;
        Ok(Engine {
            store,
            cfg: cfg.clone(),
        })
    }

    fn abs_of(&self, root: &str, rel: &str) -> std::path::PathBuf {
        for r in &self.cfg.raw.roots {
            if r.key == root {
                return self.cfg.abs_root(r).join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            }
        }
        self.cfg.dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
    }

    pub fn source(&self, root: &str, rel: &str, start: i32, end: i32, max_lines: i32) -> String {
        let path = self.abs_of(root, rel);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => return format!("<cannot read {}: {e}>", path.display()),
        };
        let max = if max_lines > 0 { max_lines } else { 200 };
        let end = end.min(start + max - 1).max(start);
        let mut out = String::new();
        for (i, line) in text.lines().enumerate() {
            let n = (i + 1) as i32;
            if n < start {
                continue;
            }
            if n > end {
                break;
            }
            out.push_str(&format!("{n:>5}| {line}\n"));
        }
        out
    }

    pub fn search_symbols(
        &self,
        query: &str,
        kind: Option<&str>,
        lang: Option<&str>,
        root: Option<&str>,
        path: Option<&str>,
        limit: i64,
    ) -> Result<Vec<Value>, String> {
        let mut sql = String::from(
            "SELECT s.id, s.kind, s.name, s.qualname, COALESCE(s.namespace,''), s.lang, s.root,
                    COALESCE(f.path,''), s.start_line, s.end_line, COALESCE(s.signature,''),
                    COALESCE(s.container,''), COALESCE(s.param_count,-1)
             FROM symbols s LEFT JOIN files f ON f.id=s.file_id
             WHERE (s.name LIKE ?1 OR s.qualname LIKE ?1)",
        );
        let pat = format!("%{query}%");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(pat)];
        if let Some(k) = kind {
            if !k.is_empty() {
                sql.push_str(&format!(" AND s.kind=?{}", args.len() + 1));
                args.push(Box::new(k.to_string()));
            }
        }
        if let Some(l) = lang {
            if !l.is_empty() {
                sql.push_str(&format!(" AND s.lang=?{}", args.len() + 1));
                args.push(Box::new(l.to_string()));
            }
        }
        if let Some(r) = root {
            if !r.is_empty() {
                sql.push_str(&format!(" AND s.root=?{}", args.len() + 1));
                args.push(Box::new(r.to_string()));
            }
        }
        if let Some(p) = path {
            if !p.is_empty() {
                sql.push_str(&format!(" AND f.path LIKE ?{}", args.len() + 1));
                args.push(Box::new(format!("%{p}%")));
            }
        }
        sql.push_str(&format!(
            " ORDER BY CASE WHEN s.name=?{} THEN 0 WHEN s.name LIKE ?{} THEN 1 ELSE 2 END,
               LENGTH(s.qualname) LIMIT ?{}",
            args.len() + 1,
            args.len() + 2,
            args.len() + 3
        ));
        args.push(Box::new(query.to_string()));
        args.push(Box::new(format!("{query}%")));
        args.push(Box::new(limit.max(1)));

        let mut stmt = self.store.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "kind": r.get::<_, String>(1)?,
                    "name": r.get::<_, String>(2)?,
                    "qualname": r.get::<_, String>(3)?,
                    "namespace": r.get::<_, String>(4)?,
                    "lang": r.get::<_, String>(5)?,
                    "root": r.get::<_, String>(6)?,
                    "path": r.get::<_, String>(7)?,
                    "start_line": r.get::<_, i32>(8)?,
                    "end_line": r.get::<_, i32>(9)?,
                    "signature": r.get::<_, String>(10)?,
                    "container": r.get::<_, String>(11)?,
                    "param_count": r.get::<_, i32>(12)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn get_symbol(&self, id: i64) -> Result<Value, String> {
        let mut stmt = self
            .store
            .conn
            .prepare(
                "SELECT s.id, s.kind, s.name, s.qualname, COALESCE(s.namespace,''), s.lang, s.root,
                        COALESCE(f.path,''), s.start_line, s.end_line, COALESCE(s.signature,''),
                        COALESCE(s.container,''), COALESCE(s.param_count,-1)
                 FROM symbols s LEFT JOIN files f ON f.id=s.file_id WHERE s.id=?1",
            )
            .map_err(|e| e.to_string())?;
        let row = stmt
            .query_row(params![id], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "kind": r.get::<_, String>(1)?,
                    "name": r.get::<_, String>(2)?,
                    "qualname": r.get::<_, String>(3)?,
                    "namespace": r.get::<_, String>(4)?,
                    "lang": r.get::<_, String>(5)?,
                    "root": r.get::<_, String>(6)?,
                    "path": r.get::<_, String>(7)?,
                    "start_line": r.get::<_, i32>(8)?,
                    "end_line": r.get::<_, i32>(9)?,
                    "signature": r.get::<_, String>(10)?,
                    "container": r.get::<_, String>(11)?,
                    "param_count": r.get::<_, i32>(12)?,
                }))
            })
            .map_err(|e| format!("symbol {id} not found: {e}"))?;
        Ok(row)
    }

    pub fn file_outline(&self, path: &str, root: Option<&str>, limit: i64) -> Result<Vec<Value>, String> {
        let mut sql = String::from(
            "SELECT s.id, s.kind, s.name, s.qualname, COALESCE(s.namespace,''), s.lang, s.root,
                    COALESCE(f.path,''), s.start_line, s.end_line, COALESCE(s.signature,''),
                    COALESCE(s.container,''), COALESCE(s.param_count,-1)
             FROM symbols s LEFT JOIN files f ON f.id=s.file_id
             WHERE f.path LIKE ?1",
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(format!("%{path}%"))];
        if let Some(r) = root {
            if !r.is_empty() {
                sql.push_str(&format!(" AND s.root=?{}", args.len() + 1));
                args.push(Box::new(r.to_string()));
            }
        }
        sql.push_str(&format!(" ORDER BY f.path, s.start_line LIMIT ?{}", args.len() + 1));
        args.push(Box::new(limit.max(1)));
        let mut stmt = self.store.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "kind": r.get::<_, String>(1)?,
                    "name": r.get::<_, String>(2)?,
                    "qualname": r.get::<_, String>(3)?,
                    "lang": r.get::<_, String>(5)?,
                    "root": r.get::<_, String>(6)?,
                    "path": r.get::<_, String>(7)?,
                    "start_line": r.get::<_, i32>(8)?,
                    "end_line": r.get::<_, i32>(9)?,
                    "signature": r.get::<_, String>(10)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn related(&self, id: i64, direction: &str, depth: i64, limit: i64) -> Result<Vec<Value>, String> {
        let depth = depth.clamp(1, 4);
        let limit = limit.max(1);
        let mut seen = std::collections::HashSet::new();
        seen.insert(id);
        let mut current = vec![id];
        let mut out = Vec::new();
        for d in 1..=depth {
            let mut next = Vec::new();
            for cur in &current {
                let sql = if direction == "in" {
                    "SELECT s.id, s.kind, s.name, s.qualname, COALESCE(s.namespace,''), s.lang, s.root,
                            COALESCE(f.path,''), s.start_line, s.end_line, COALESCE(s.signature,''),
                            MIN(e.kind), MIN(COALESCE(e.line,0))
                     FROM edges e JOIN symbols s ON s.id=e.src LEFT JOIN files f ON f.id=s.file_id
                     WHERE e.dst=?1 GROUP BY s.id LIMIT ?2"
                } else {
                    "SELECT s.id, s.kind, s.name, s.qualname, COALESCE(s.namespace,''), s.lang, s.root,
                            COALESCE(f.path,''), s.start_line, s.end_line, COALESCE(s.signature,''),
                            MIN(e.kind), MIN(COALESCE(e.line,0))
                     FROM edges e JOIN symbols s ON s.id=e.dst LEFT JOIN files f ON f.id=s.file_id
                     WHERE e.src=?1 GROUP BY s.id LIMIT ?2"
                };
                let mut stmt = self.store.conn.prepare(sql).map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map(params![cur, limit], |r| {
                        Ok(json!({
                            "id": r.get::<_, i64>(0)?,
                            "kind": r.get::<_, String>(1)?,
                            "name": r.get::<_, String>(2)?,
                            "qualname": r.get::<_, String>(3)?,
                            "namespace": r.get::<_, String>(4)?,
                            "lang": r.get::<_, String>(5)?,
                            "root": r.get::<_, String>(6)?,
                            "path": r.get::<_, String>(7)?,
                            "start_line": r.get::<_, i32>(8)?,
                            "end_line": r.get::<_, i32>(9)?,
                            "signature": r.get::<_, String>(10)?,
                            "edge_kind": r.get::<_, String>(11)?,
                            "line": r.get::<_, i32>(12)?,
                            "depth": d,
                        }))
                    })
                    .map_err(|e| e.to_string())?;
                for row in rows {
                    let v = row.map_err(|e| e.to_string())?;
                    let rid = v["id"].as_i64().unwrap_or(0);
                    if seen.insert(rid) {
                        next.push(rid);
                        out.push(v);
                    }
                    if out.len() as i64 >= limit {
                        return Ok(out);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            current = next;
        }
        Ok(out)
    }

    pub fn refs_to(&self, id: i64, limit: i64) -> Result<Vec<Value>, String> {
        self.related(id, "in", 1, limit)
    }

    pub fn impact(&self, id: Option<i64>, path: Option<&str>, depth: i64) -> Result<Value, String> {
        let symbol_id = match id {
            Some(v) if v > 0 => v,
            _ => {
                let p = path.ok_or_else(|| "impact needs id or path".to_string())?;
                let hits = self.file_outline(p, None, 2000)?;
                hits.first()
                    .and_then(|v| v["id"].as_i64())
                    .ok_or_else(|| format!("no symbol in {p}"))?
            }
        };
        let subject = self
            .get_symbol(symbol_id)
            .ok()
            .and_then(|v| v["qualname"].as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let kind = self
            .get_symbol(symbol_id)
            .ok()
            .and_then(|v| v["kind"].as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        let related = self.related(symbol_id, "in", depth, 1000)?;
        Ok(json!({
            "subject": subject,
            "kind": kind,
            "affected": related,
        }))
    }

    pub fn asset_search(&self, name: &str, kind: &str, root: &str, path: &str, limit: i64) -> Result<Vec<Value>, String> {
        let mut sql = String::from(
            "SELECT asset_key, kind, COALESCE(root,''), COALESCE(path,''), COALESCE(name,''),
                    COALESCE(unity_id,''), COALESCE(meta,'{}') FROM assets WHERE 1=1",
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if !name.is_empty() {
            args.push(Box::new(format!("%{name}%")));
            args.push(Box::new(format!("%{name}%")));
            sql.push_str(&format!(
                " AND (name LIKE ?{} OR asset_key LIKE ?{})",
                args.len() - 1,
                args.len()
            ));
        }
        if !kind.is_empty() {
            args.push(Box::new(kind.to_string()));
            sql.push_str(&format!(" AND kind=?{}", args.len()));
        }
        if !root.is_empty() {
            args.push(Box::new(root.to_string()));
            sql.push_str(&format!(" AND root=?{}", args.len()));
        }
        if !path.is_empty() {
            args.push(Box::new(format!("%{path}%")));
            sql.push_str(&format!(" AND path LIKE ?{}", args.len()));
        }
        args.push(Box::new(limit.max(1)));
        sql.push_str(&format!(" ORDER BY path, name LIMIT ?{}", args.len()));
        let mut stmt = self.store.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), |r| {
                let meta: String = r.get(6)?;
                Ok(json!({
                    "key": r.get::<_, String>(0)?,
                    "kind": r.get::<_, String>(1)?,
                    "root": r.get::<_, String>(2)?,
                    "path": r.get::<_, String>(3)?,
                    "name": r.get::<_, String>(4)?,
                    "unity_id": r.get::<_, String>(5)?,
                    "meta": serde_json::from_str::<Value>(&meta).unwrap_or(json!({})),
                }))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn asset_info(&self, key: &str, path: &str, limit: i64) -> Result<Value, String> {
        let mut stmt = self
            .store
            .conn
            .prepare(
                "SELECT asset_key, kind, COALESCE(root,''), COALESCE(path,''), COALESCE(name,''),
                        COALESCE(unity_id,''), COALESCE(meta,'{}') FROM assets
                 WHERE (?1 <> '' AND asset_key=?1) OR (?1 = '' AND path LIKE ?2)
                 ORDER BY LENGTH(path) LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        let asset = stmt
            .query_row(params![key, format!("%{path}%")], |r| {
                let meta: String = r.get(6)?;
                Ok(json!({
                    "key": r.get::<_, String>(0)?,
                    "kind": r.get::<_, String>(1)?,
                    "root": r.get::<_, String>(2)?,
                    "path": r.get::<_, String>(3)?,
                    "name": r.get::<_, String>(4)?,
                    "unity_id": r.get::<_, String>(5)?,
                    "meta": serde_json::from_str::<Value>(&meta).unwrap_or(json!({})),
                }))
            })
            .map_err(|e| format!("asset not found: {e}"))?;
        let k = asset["key"].as_str().unwrap_or("").to_string();
        let out_edges = self.asset_edges(&k, "out", limit)?;
        let in_edges = self.asset_edges(&k, "in", limit)?;
        Ok(json!({ "asset": asset, "edges_out": out_edges, "edges_in": in_edges }))
    }

    pub fn asset_edges(&self, key: &str, direction: &str, limit: i64) -> Result<Vec<Value>, String> {
        let sql = if direction == "in" {
            "SELECT e.src, e.dst, e.kind, COALESCE(e.detail,''), COALESCE(e.line,0),
                    COALESCE(a.name,''), COALESCE(b.name,''), COALESCE(a.path,''), COALESCE(b.path,'')
             FROM asset_edges e LEFT JOIN assets a ON a.asset_key=e.src
             LEFT JOIN assets b ON b.asset_key=e.dst WHERE e.dst=?1 LIMIT ?2"
        } else {
            "SELECT e.src, e.dst, e.kind, COALESCE(e.detail,''), COALESCE(e.line,0),
                    COALESCE(a.name,''), COALESCE(b.name,''), COALESCE(a.path,''), COALESCE(b.path,'')
             FROM asset_edges e LEFT JOIN assets a ON a.asset_key=e.src
             LEFT JOIN assets b ON b.asset_key=e.dst WHERE e.src=?1 LIMIT ?2"
        };
        let mut stmt = self.store.conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![key, limit.max(1)], |r| {
                Ok(json!({
                    "src": r.get::<_, String>(0)?,
                    "dst": r.get::<_, String>(1)?,
                    "kind": r.get::<_, String>(2)?,
                    "detail": r.get::<_, String>(3)?,
                    "line": r.get::<_, i32>(4)?,
                    "src_name": r.get::<_, String>(5)?,
                    "dst_name": r.get::<_, String>(6)?,
                    "src_path": r.get::<_, String>(7)?,
                    "dst_path": r.get::<_, String>(8)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    pub fn guid_lookup(&self, sel: &str, limit: i64) -> Result<Vec<Value>, String> {
        let sel = sel.trim().trim_start_matches("guid:").to_ascii_lowercase();
        let mut stmt = self
            .store
            .conn
            .prepare(
                "SELECT guid, path, COALESCE(type,''), COALESCE(name,'') FROM guid_map
                 WHERE (?1 <> '' AND guid=?1) OR (?1 = '' AND path LIKE ?2)
                 ORDER BY LENGTH(path) LIMIT ?3",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![sel, format!("%{}%", sel), limit.max(1)], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            let (guid, path, ty, name) = row.map_err(|e| e.to_string())?;
            let key = format!("guid:{guid}");
            let referenced_by = self.asset_edges(&key, "in", limit)?;
            out.push(json!({
                "guid": guid, "path": path, "type": ty, "name": name,
                "referenced_by": referenced_by,
            }));
        }
        Ok(out)
    }

    pub fn health(&self, _root: &str) -> Result<Value, String> {
        let q = |sql: &str| -> i64 {
            self.store
                .conn
                .query_row(sql, [], |r| r.get::<_, i64>(0))
                .unwrap_or(0)
        };
        let files = q("SELECT COUNT(*) FROM files");
        let symbols = q("SELECT COUNT(*) FROM symbols");
        let refs = q("SELECT COUNT(*) FROM refs");
        let edges = q("SELECT COUNT(*) FROM edges");
        let unresolved = q("SELECT COUNT(*) FROM refs WHERE COALESCE(target,0)=0");
        let assets = q("SELECT COUNT(*) FROM assets");
        let asset_edges = q("SELECT COUNT(*) FROM asset_edges");
        let guid_entries = q("SELECT COUNT(*) FROM guid_map");
        let guid_refs = q("SELECT COUNT(*) FROM (SELECT DISTINCT dst FROM asset_edges WHERE dst LIKE 'guid:%')");
        let guid_mapped = q(
            "SELECT COUNT(*) FROM (SELECT DISTINCT dst FROM asset_edges e
             WHERE dst LIKE 'guid:%' AND EXISTS (SELECT 1 FROM guid_map g WHERE g.guid=SUBSTR(e.dst,6)))",
        );
        let db_bytes = std::fs::metadata(self.cfg.db_path()).map(|m| m.len()).unwrap_or(0);
        Ok(json!({
            "files": files, "symbols": symbols, "refs": refs, "edges": edges,
            "unresolved_refs": unresolved, "assets": assets, "asset_edges": asset_edges,
            "guid_map_entries": guid_entries, "guid_refs": guid_refs, "guid_refs_mapped": guid_mapped,
            "db_bytes": db_bytes,
            "indexed_at": self.store.get_meta("indexed_at"),
            "profile": self.cfg.raw.profile,
            "languages": self.cfg.languages(),
        }))
    }

    pub fn search_pattern(
        &self,
        pattern: &str,
        glob: &str,
        root: &str,
        max_results: i64,
        context: i64,
    ) -> Result<Vec<Value>, String> {
        let re = regex::Regex::new(pattern).map_err(|e| format!("invalid regex: {e}"))?;
        let mut sql = String::from("SELECT root, path FROM files WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if !root.is_empty() {
            args.push(Box::new(root.to_string()));
            sql.push_str(&format!(" AND root=?{}", args.len()));
        }
        if !glob.is_empty() {
            args.push(Box::new(format!("%{}%", glob.replace('*', "%"))));
            sql.push_str(&format!(" AND path LIKE ?{}", args.len()));
        }
        sql.push_str(" ORDER BY path");
        let mut stmt = self.store.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let refs_args: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs_args.as_slice(), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            let (r, p) = row.map_err(|e| e.to_string())?;
            let path = self.abs_of(&r, &p);
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if !re.is_match(line) {
                    continue;
                }
                let lo = (i as i64 - context).max(0) as usize;
                let hi = ((i as i64 + context + 1) as usize).min(lines.len());
                let ctx: Vec<String> = (lo..hi)
                    .map(|k| format!("{}| {}", k + 1, lines[k]))
                    .collect();
                out.push(json!({
                    "root": r, "path": p, "line": i + 1,
                    "text": line.trim_end(),
                    "context": ctx,
                }));
                if out.len() as i64 >= max_results {
                    return Ok(out);
                }
            }
        }
        Ok(out)
    }

    pub fn file_exists(&self, root: &str, rel: &str) -> bool {
        Path::new(&self.abs_of(root, rel)).is_file()
    }
}
