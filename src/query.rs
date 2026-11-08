use crate::config::Config;
use crate::store::Store;
use rusqlite::params;
use serde_json::{json, Value};
use std::path::Path;

pub struct Engine {
    pub store: Store,
    pub cfg: Config,
}

const AUDIO_GRAPH_KINDS: &[&str] = &[
    "audio",
    "audio_mixer",
    "audio_mixer_group",
    "audio_mixer_snapshot",
    "audio_mixer_effect",
    "wwise_workunit",
    "wwise_project",
    "wwise_event",
    "wwise_switch_group",
    "wwise_state_group",
    "wwise_rtpc",
    "wwise_bank",
    "wwise_aux_bus",
    "wwise_audio_device",
    "wwise_effect",
    "wwise_external_source",
    "wwise_object",
    "wwise_media",
    "fmod_project",
    "fmod_strings",
    "fmod_event",
    "fmod_bus",
    "fmod_vca",
    "fmod_bank",
];

const ART_GRAPH_KINDS: &[&str] = &[
    "gltf_scene",
    "gltf_node",
    "gltf_mesh",
    "gltf_material",
    "gltf_texture",
    "gltf_image",
    "gltf_animation",
    "gltf_skin",
    "aseprite",
    "aseprite_layer",
    "aseprite_tag",
    "fbx",
    "upm_manifest",
    "upm_lock",
    "upm_package",
    "image",
    "material",
    "font",
    "shader",
];

impl Engine {
    pub fn open(cfg: &Config) -> Result<Engine, String> {
        let store = Store::open(&cfg.db_path(), false)?;
        if store.is_stale().unwrap_or(false) {
            return Err("index schema is stale: run `codegraph233 index` to rebuild".into());
        }
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
                            MIN(e.kind), MIN(COALESCE(e.line,0)), MIN(COALESCE(e.confidence,0))
                     FROM edges e JOIN symbols s ON s.id=e.src LEFT JOIN files f ON f.id=s.file_id
                     WHERE e.dst=?1 GROUP BY s.id LIMIT ?2"
                } else {
                    "SELECT s.id, s.kind, s.name, s.qualname, COALESCE(s.namespace,''), s.lang, s.root,
                            COALESCE(f.path,''), s.start_line, s.end_line, COALESCE(s.signature,''),
                            MIN(e.kind), MIN(COALESCE(e.line,0)), MIN(COALESCE(e.confidence,0))
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
                            "confidence": crate::store::conf_text(r.get::<_, i64>(13)?),
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

    pub fn audio_graph(
        &self,
        root: Option<&str>,
        asset: Option<&str>,
        limit: i64,
    ) -> Result<Value, String> {
        self.pipeline_graph("audio", AUDIO_GRAPH_KINDS, root, asset, limit)
    }

    pub fn art_graph(
        &self,
        root: Option<&str>,
        asset: Option<&str>,
        limit: i64,
    ) -> Result<Value, String> {
        self.pipeline_graph("art", ART_GRAPH_KINDS, root, asset, limit)
    }

    fn pipeline_graph(
        &self,
        domain: &str,
        kinds: &[&str],
        root: Option<&str>,
        asset: Option<&str>,
        limit: i64,
    ) -> Result<Value, String> {
        let limit = limit.clamp(1, 2000);
        let placeholders = vec!["?"; kinds.len()].join(",");
        let mut sql = format!(
            "SELECT asset_key, kind, path, COALESCE(name,'') FROM assets WHERE kind IN ({placeholders})"
        );
        let mut params_v: Vec<String> = kinds.iter().map(|s| s.to_string()).collect();
        if let Some(r) = root {
            sql.push_str(" AND root = ?");
            params_v.push(r.to_string());
        }
        if let Some(a) = asset {
            sql.push_str(" AND (asset_key = ? OR path = ?)");
            params_v.push(a.to_string());
            params_v.push(a.to_string());
        }
        sql.push_str(&format!(" ORDER BY kind, path LIMIT {limit}"));

        let mut stmt = self.store.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params_v.iter().map(|s| s.as_str())),
                |r| {
                    Ok(json!({
                        "key": r.get::<_, String>(0)?,
                        "kind": r.get::<_, String>(1)?,
                        "path": r.get::<_, String>(2)?,
                        "name": r.get::<_, String>(3)?,
                    }))
                },
            )
            .map_err(|e| e.to_string())?;
        let mut assets = Vec::new();
        for row in rows {
            assets.push(row.map_err(|e| e.to_string())?);
        }
        let keys: Vec<String> = assets
            .iter()
            .filter_map(|a| a["key"].as_str().map(|s| s.to_string()))
            .collect();
        let mut edges = Vec::new();
        if !keys.is_empty() {
            let ph = vec!["?"; keys.len()].join(",");
            let esql = format!(
                "SELECT e.src, e.dst, e.kind, COALESCE(e.detail,''), COALESCE(e.line,0),
                        COALESCE(a.name,''), COALESCE(b.name,'')
                 FROM asset_edges e LEFT JOIN assets a ON a.asset_key=e.src
                 LEFT JOIN assets b ON b.asset_key=e.dst
                 WHERE e.src IN ({ph}) OR e.dst IN ({ph})
                 ORDER BY e.src, e.kind, e.dst LIMIT {limit}"
            );
            let mut stmt = self.store.conn.prepare(&esql).map_err(|e| e.to_string())?;
            let iter = keys
                .iter()
                .map(|s| s.as_str())
                .chain(keys.iter().map(|s| s.as_str()));
            let rows = stmt
                .query_map(rusqlite::params_from_iter(iter), |r| {
                    Ok(json!({
                        "src": r.get::<_, String>(0)?,
                        "dst": r.get::<_, String>(1)?,
                        "kind": r.get::<_, String>(2)?,
                        "detail": r.get::<_, String>(3)?,
                        "line": r.get::<_, i32>(4)?,
                        "src_name": r.get::<_, String>(5)?,
                        "dst_name": r.get::<_, String>(6)?,
                    }))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                edges.push(row.map_err(|e| e.to_string())?);
            }
        }
        let mut by_kind: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for a in &assets {
            if let Some(k) = a["kind"].as_str() {
                *by_kind.entry(k.to_string()).or_insert(0) += 1;
            }
        }
        Ok(json!({
            "domain": domain,
            "root": root,
            "asset_count": assets.len(),
            "edge_count": edges.len(),
            "kinds": by_kind,
            "assets": assets,
            "edges": edges,
        }))
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
            let mut referenced_by = self.asset_edges(&key, "in", limit)?;
            // Resolved guids were rewritten to the asset path by materialization.
            if let Ok(resolved) = self.store.conn.query_row(
                "SELECT asset_key FROM assets WHERE path=?1 ORDER BY LENGTH(asset_key) LIMIT 1",
                params![path],
                |r| r.get::<_, String>(0),
            ) {
                if resolved != key {
                    for e in self.asset_edges(&resolved, "in", limit)? {
                        if !referenced_by.contains(&e) {
                            referenced_by.push(e);
                        }
                    }
                }
            }
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
        let unresolved: i64 = self
            .store
            .get_meta("unresolved_refs")
            .parse()
            .unwrap_or_else(|_| q("SELECT COUNT(*) FROM refs WHERE COALESCE(target,0)=0"));
        let assets = q("SELECT COUNT(*) FROM assets");
        let asset_edges = q("SELECT COUNT(*) FROM asset_edges");
        let guid_entries = q("SELECT COUNT(*) FROM guid_map");
        let meta_i = |key: &str| -> Option<i64> { self.store.get_meta(key).parse().ok() };
        let guid_refs = meta_i("guid_refs").unwrap_or_else(|| {
            q("SELECT COUNT(*) FROM (SELECT DISTINCT dst FROM asset_edges WHERE dst LIKE 'guid:%')")
        });
        let guid_mapped = meta_i("guid_refs_mapped").unwrap_or_else(|| {
            q(
                "SELECT COUNT(*) FROM (SELECT DISTINCT dst FROM asset_edges e
                 WHERE dst LIKE 'guid:%' AND EXISTS (SELECT 1 FROM guid_map g WHERE g.guid=SUBSTR(e.dst,6)))",
            )
        });
        let db_bytes = std::fs::metadata(self.cfg.db_path()).map(|m| m.len()).unwrap_or(0);
        let mut diagnostics = Vec::new();
        let mut statement = self.store.conn.prepare("SELECT value FROM meta WHERE key LIKE 'diagnostic:%' ORDER BY key LIMIT 32").map_err(|e| e.to_string())?;
        for value in statement.query_map([], |r| r.get::<_,String>(0)).map_err(|e| e.to_string())? {
            if let Ok(value) = serde_json::from_str::<Value>(&value.map_err(|e| e.to_string())?) { diagnostics.push(value); }
        }
        Ok(json!({
            "files": files, "symbols": symbols, "refs": refs, "edges": edges,
            "unresolved_refs": unresolved, "assets": assets, "asset_edges": asset_edges,
            "guid_map_entries": guid_entries, "guid_refs": guid_refs, "guid_refs_mapped": guid_mapped,
            "db_bytes": db_bytes,
            "indexed_at": self.store.get_meta("indexed_at"),
            "profile": self.cfg.raw.profile,
            "languages": self.cfg.languages(),
            "reader_version":env!("CARGO_PKG_VERSION"),
            "parser_version":self.store.get_meta("parser_version"),
            "parse_errors":self.store.get_meta("parse_errors").parse::<usize>().unwrap_or(0),
            "diagnostics_first_32":diagnostics,
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

    // ------------------------------------------------------------- overview

    /// One-call orientation for an agent: what is indexed, in what languages,
    /// how the code is grouped, and what game assets exist.
    pub fn overview(&self, root: &str) -> Result<Value, String> {
        let grouped = |sql: &str| -> Vec<(String, i64)> {
            let mut out = Vec::new();
            let Ok(mut stmt) = self.store.conn.prepare(sql) else {
                return out;
            };
            if let Ok(rows) = stmt.query_map(params![root], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            }) {
                for row in rows.flatten() {
                    out.push(row);
                }
            }
            out
        };
        let pairs = |rows: Vec<(String, i64)>| -> Value {
            Value::Array(
                rows.into_iter()
                    .map(|(k, v)| json!({"name": k, "count": v}))
                    .collect(),
            )
        };
        let langs = grouped(&format!(
            "SELECT lang, COUNT(*) FROM files WHERE (?1='' OR root=?1) GROUP BY lang ORDER BY 2 DESC"
        ));
        let kinds = grouped(&format!(
            "SELECT kind, COUNT(*) FROM symbols WHERE (?1='' OR root=?1) GROUP BY kind ORDER BY 2 DESC"
        ));
        let namespaces = grouped(&format!(
            "SELECT COALESCE(NULLIF(namespace,''),'(global)') AS ns, COUNT(*) FROM symbols
             WHERE (?1='' OR root=?1) GROUP BY ns ORDER BY 2 DESC LIMIT 20"
        ));
        let asset_kinds = grouped(&format!(
            "SELECT kind, COUNT(*) FROM assets WHERE (?1='' OR root=?1) GROUP BY kind ORDER BY 2 DESC"
        ));
        let counts = self.health(root)?;
        let mut hints: Vec<&str> = Vec::new();
        if asset_kinds.iter().any(|(k, _)| k == "prefab") {
            hints.push("use cg233_asset_tree / cg233_asset_refs for prefab hierarchies");
        }
        if asset_kinds.iter().any(|(k, _)| k == "shader") {
            hints.push("use cg233_shader_info for properties/includes/keywords");
        }
        if asset_kinds.iter().any(|(k, _)| k == "config_table") {
            hints.push("use cg233_config_tables / cg233_config_readers for data tables");
        }
        hints.push("use cg233_find_symbol then cg233_call_graph / cg233_impact");
        Ok(json!({
            "counts": counts,
            "languages": pairs(langs),
            "symbol_kinds": pairs(kinds),
            "top_namespaces": pairs(namespaces),
            "asset_kinds": pairs(asset_kinds),
            "next": hints,
        }))
    }

    /// Namespace → owning types → symbol chain for one declaration.
    pub fn symbol_path(&self, id: i64) -> Result<Value, String> {
        let sym = self.get_symbol(id)?;
        let ns = sym["namespace"].as_str().unwrap_or("");
        let container = sym["container"].as_str().unwrap_or("");
        let mut chain: Vec<Value> = Vec::new();
        if !ns.is_empty() {
            let mut acc = String::new();
            for seg in ns.split('.') {
                if seg.is_empty() {
                    continue;
                }
                if !acc.is_empty() {
                    acc.push('.');
                }
                acc.push_str(seg);
                chain.push(json!({"kind": "namespace", "name": seg, "qualname": acc}));
            }
        }
        let mut owner = container.to_string();
        if !ns.is_empty() {
            if let Some(rest) = owner.strip_prefix(&format!("{ns}.")) {
                owner = rest.to_string();
            }
        }
        if !owner.is_empty() && owner != sym["qualname"].as_str().unwrap_or("") {
            let mut qual = String::new();
            for seg in owner.split('.') {
                if seg.is_empty() {
                    continue;
                }
                if !qual.is_empty() {
                    qual.push('.');
                }
                qual.push_str(seg);
                let full = if ns.is_empty() {
                    qual.clone()
                } else {
                    format!("{ns}.{qual}")
                };
                chain.push(json!({"kind": "type", "name": seg, "qualname": full}));
            }
        }
        chain.push(json!({
            "kind": sym["kind"], "name": sym["name"], "qualname": sym["qualname"],
            "id": sym["id"], "path": sym["path"], "line": sym["start_line"],
        }));
        Ok(json!({"symbol": sym, "path": chain}))
    }

    // ---------------------------------------------------------------- assets

    fn find_asset(&self, key: &str, path: &str, kinds: &[&str]) -> Result<Value, String> {
        let kind_clause = if kinds.is_empty() {
            String::new()
        } else {
            format!(
                " AND kind IN ({})",
                kinds
                    .iter()
                    .map(|k| format!("'{k}'"))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        let sql = format!(
            "SELECT asset_key, kind, COALESCE(root,''), COALESCE(path,''), COALESCE(name,''),
                    COALESCE(unity_id,''), COALESCE(meta,'{{}}') FROM assets
             WHERE ((?1 <> '' AND asset_key=?1) OR (?1 = '' AND path LIKE ?2)){kind_clause}
             ORDER BY LENGTH(path) LIMIT 1"
        );
        let mut stmt = self.store.conn.prepare(&sql).map_err(|e| e.to_string())?;
        stmt.query_row(params![key, format!("%{path}%")], |r| {
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
        .map_err(|e| format!("asset not found: {e}"))
    }

    /// Prefab/scene GameObject hierarchy with the script/asset edges of each node.
    pub fn asset_tree(&self, key: &str, path: &str, limit: i64) -> Result<Value, String> {
        let root_asset = self.find_asset(key, path, &["prefab", "scene"])?;
        let root_key = root_asset["key"].as_str().unwrap_or("").to_string();
        let doc_path = root_asset["path"].as_str().unwrap_or("").to_string();
        let limit = limit.clamp(1, 20_000) as usize;
        // Child → parent map for every GameObject in this file, so a node that
        // is both a file child and nested under another object reports its
        // real parent.
        let mut parent_of: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if let Ok(mut stmt) = self.store.conn.prepare(
            "SELECT e.dst, e.src FROM asset_edges e JOIN assets a ON a.asset_key=e.dst
             WHERE a.path=?1 AND e.kind='contains' AND COALESCE(e.detail,'')='child'",
        ) {
            if let Ok(rows) = stmt.query_map(params![doc_path], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            }) {
                for row in rows.flatten() {
                    parent_of.insert(row.0, row.1);
                }
            }
        }
        let mut nodes: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
        let mut children: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        let mut queue: Vec<(String, i32)> = vec![(root_key.clone(), 0)];
        let mut order: Vec<String> = Vec::new();
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        visited.insert(root_key.clone());
        while let Some((cur, depth)) = queue.pop() {
            if order.len() >= limit {
                break;
            }
            let info: Option<(String, String, String, String)> = self
                .store
                .conn
                .query_row(
                    "SELECT kind, COALESCE(name,''), COALESCE(unity_id,''), COALESCE(path,'')
                     FROM assets WHERE asset_key=?1",
                    params![cur],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .ok();
            let (kind, name, unity_id, apath) = info.unwrap_or_default();
            let mut stmt = self
                .store
                .conn
                .prepare(
                    "SELECT e.dst, e.kind, COALESCE(b.name,''), COALESCE(b.path,'')
                     FROM asset_edges e LEFT JOIN assets b ON b.asset_key=e.dst
                     WHERE e.src=?1 AND e.kind='contains' LIMIT ?2",
                )
                .map_err(|e| e.to_string())?;
            let kids: Vec<(String, String, String, String)> = stmt
                .query_map(params![cur, limit as i64], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .map_err(|e| e.to_string())?
                .flatten()
                .collect();
            let mut uses: Vec<Value> = Vec::new();
            let mut ustmt = self
                .store
                .conn
                .prepare(
                    "SELECT e.kind, e.dst, COALESCE(b.name,''), COALESCE(b.path,''), COALESCE(e.detail,'')
                     FROM asset_edges e LEFT JOIN assets b ON b.asset_key=e.dst
                     WHERE e.src=?1 AND e.kind<>'contains' LIMIT 40",
                )
                .map_err(|e| e.to_string())?;
            if let Ok(rows) = ustmt.query_map(params![cur], |r| {
                Ok(json!({
                    "kind": r.get::<_, String>(0)?,
                    "target": r.get::<_, String>(1)?,
                    "target_name": r.get::<_, String>(2)?,
                    "target_path": r.get::<_, String>(3)?,
                    "detail": r.get::<_, String>(4)?,
                }))
            }) {
                uses.extend(rows.flatten());
            }
            nodes.insert(
                cur.clone(),
                json!({
                    "key": cur, "kind": kind, "name": name, "unity_id": unity_id,
                    "path": apath, "depth": depth, "parent": parent_of.get(&cur).cloned().unwrap_or_default(),
                    "uses": uses,
                }),
            );
            order.push(cur.clone());
            let mut ids = Vec::new();
            for (dst, _k, _n, _p) in kids {
                ids.push(dst.clone());
                if visited.insert(dst.clone()) {
                    queue.push((dst, depth + 1));
                }
            }
            if !ids.is_empty() {
                children.insert(cur, ids);
            }
        }
        let root = nodes.get(&root_key).cloned().unwrap_or(json!({}));
        Ok(json!({
            "root": root,
            "nodes": nodes,
            "children": children,
            "node_count": order.len(),
            "truncated": order.len() >= limit,
        }))
    }

    /// Counts and byte totals per asset kind, plus the largest assets.
    pub fn asset_stats(&self, root: &str, limit: i64) -> Result<Value, String> {
        let mut out_kinds: Vec<Value> = Vec::new();
        let sql = "SELECT kind, COUNT(*), SUM(COALESCE(CAST(json_extract(COALESCE(meta,'{}'),'$.bytes') AS INTEGER),0))
             FROM assets WHERE path <> '' AND (?1='' OR root=?1) GROUP BY kind ORDER BY 3 DESC";
        let mut stmt = self.store.conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![root], |r| {
                Ok(json!({
                    "kind": r.get::<_, String>(0)?,
                    "count": r.get::<_, i64>(1)?,
                    "bytes": r.get::<_, i64>(2)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            out_kinds.push(row.map_err(|e| e.to_string())?);
        }
        let largest_sql = "SELECT kind, COALESCE(name,''), COALESCE(path,''),
                    COALESCE(CAST(json_extract(COALESCE(meta,'{}'),'$.bytes') AS INTEGER),0) AS bytes
             FROM assets WHERE path <> '' AND (?1='' OR root=?1)
             ORDER BY bytes DESC LIMIT ?2";
        let mut stmt = self.store.conn.prepare(largest_sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![root, limit.max(1)], |r| {
                Ok(json!({
                    "kind": r.get::<_, String>(0)?,
                    "name": r.get::<_, String>(1)?,
                    "path": r.get::<_, String>(2)?,
                    "bytes": r.get::<_, i64>(3)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        let mut largest: Vec<Value> = Vec::new();
        for row in rows {
            largest.push(row.map_err(|e| e.to_string())?);
        }
        let total: i64 = self
            .store
            .conn
            .query_row(
                "SELECT COALESCE(SUM(COALESCE(CAST(json_extract(COALESCE(meta,'{}'),'$.bytes') AS INTEGER),0)),0)
                 FROM assets WHERE path <> '' AND (?1='' OR root=?1)",
                params![root],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok(json!({"by_kind": out_kinds, "largest": largest, "total_bytes": total}))
    }

    /// File-level assets nothing references, largest first.
    pub fn asset_orphans(
        &self,
        root: &str,
        kind: &str,
        limit: i64,
    ) -> Result<Vec<Value>, String> {
        let mut sql = String::from(
            "SELECT a.kind, COALESCE(a.name,''), COALESCE(a.path,''),
                    COALESCE(a.asset_key,''),
                    COALESCE(CAST(json_extract(COALESCE(a.meta,'{}'),'$.bytes') AS INTEGER),0) AS bytes
             FROM assets a
             WHERE a.path <> '' AND a.kind NOT IN
               ('prefab_object','scene_object','guid_ref','shader_property','meta','scriptable_asset')
               AND NOT EXISTS (SELECT 1 FROM asset_edges e WHERE e.dst = a.asset_key)",
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if !root.is_empty() {
            args.push(Box::new(root.to_string()));
            sql.push_str(&format!(" AND a.root=?{}", args.len()));
        }
        if !kind.is_empty() {
            args.push(Box::new(kind.to_string()));
            sql.push_str(&format!(" AND a.kind=?{}", args.len()));
        }
        args.push(Box::new(limit.max(1)));
        sql.push_str(&format!(" ORDER BY bytes DESC, a.path LIMIT ?{}", args.len()));
        let mut stmt = self.store.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(refs.as_slice(), |r| {
                Ok(json!({
                    "kind": r.get::<_, String>(0)?,
                    "name": r.get::<_, String>(1)?,
                    "path": r.get::<_, String>(2)?,
                    "key": r.get::<_, String>(3)?,
                    "bytes": r.get::<_, i64>(4)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// ShaderLab view: properties, includes, keywords, HLSL functions, users.
    pub fn shader_info(&self, key: &str, path: &str, limit: i64) -> Result<Value, String> {
        let asset = self.find_asset(key, path, &["shader"])?;
        let k = asset["key"].as_str().unwrap_or("").to_string();
        let shader_path = asset["path"].as_str().unwrap_or("").to_string();
        let mut stmt = self
            .store
            .conn
            .prepare(
                "SELECT COALESCE(name,''), COALESCE(unity_id,'') FROM assets
                 WHERE path=?1 AND kind='shader_property' ORDER BY name LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let properties: Vec<Value> = stmt
            .query_map(params![shader_path, limit.max(1)], |r| {
                Ok(json!({"name": r.get::<_, String>(0)?, "id": r.get::<_, String>(1)?}))
            })
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();
        let includes: Vec<Value> = self
            .asset_edges(&k, "out", limit)?
            .into_iter()
            .filter(|e| e["kind"] == "includes")
            .collect();
        let users: Vec<Value> = self
            .asset_edges(&k, "in", limit)?
            .into_iter()
            .filter(|e| e["kind"] != "is_variant_of")
            .collect();
        let mut stmt = self
            .store
            .conn
            .prepare(
                "SELECT s.id, s.name, s.qualname, s.start_line FROM symbols s
                 LEFT JOIN files f ON f.id=s.file_id
                 WHERE s.lang='shader' AND f.path=?1 ORDER BY s.start_line LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let functions: Vec<Value> = stmt
            .query_map(params![shader_path, limit.max(1)], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?,
                    "qualname": r.get::<_, String>(2)?, "line": r.get::<_, i32>(3)?,
                }))
            })
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();
        Ok(json!({
            "shader": asset,
            "properties": properties,
            "includes": includes,
            "used_by": users,
            "functions": functions,
            "keywords": asset["meta"]["keywords"].clone(),
        }))
    }

    /// Config tables (xlsx sheets, csv/tsv, json) with headers and row counts.
    pub fn config_tables(
        &self,
        root: &str,
        path: &str,
        limit: i64,
    ) -> Result<Vec<Value>, String> {
        let mut sql = String::from(
            "SELECT asset_key, COALESCE(name,''), COALESCE(path,''), COALESCE(root,''), COALESCE(meta,'{}')
             FROM assets WHERE kind='config_table'",
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
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
                let meta: String = r.get(4)?;
                Ok(json!({
                    "key": r.get::<_, String>(0)?,
                    "name": r.get::<_, String>(1)?,
                    "path": r.get::<_, String>(2)?,
                    "root": r.get::<_, String>(3)?,
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

    /// Code locations that mention a config table (file name, sheet name or
    /// table name). This is a text-level probe: it finds load/hot-reload sites
    /// without pretending to be a data-flow analysis.
    pub fn config_readers(
        &self,
        table: &str,
        root: &str,
        limit: i64,
    ) -> Result<Value, String> {
        let tables = self.config_tables(root, table, 20)?;
        if tables.is_empty() {
            return Ok(json!({"tables": [], "readers": [], "note": "no matching config table"}));
        }
        let mut needles: Vec<String> = Vec::new();
        for t in &tables {
            if let Some(p) = t["path"].as_str() {
                if let Some(name) = Path::new(p).file_stem() {
                    needles.push(name.to_string_lossy().to_string());
                }
            }
            if let Some(n) = t["name"].as_str() {
                if !n.is_empty() {
                    needles.push(n.to_string());
                }
            }
            if let Some(headers) = t["meta"]["headers"].as_str() {
                if !headers.is_empty() {
                    needles.push(headers.split(',').next().unwrap_or("").to_string());
                }
            }
        }
        needles.sort();
        needles.dedup();
        needles.retain(|n| n.len() >= 3);
        let language_list = crate::config::CODE_LANGUAGES
            .iter()
            .map(|lang| format!("'{lang}'"))
            .collect::<Vec<_>>()
            .join(",");
        let mut sql = format!(
            "SELECT root, path FROM files WHERE lang IN ({language_list})"
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if !root.is_empty() {
            args.push(Box::new(root.to_string()));
            sql.push_str(&format!(" AND root=?{}", args.len()));
        }
        sql.push_str(" ORDER BY path");
        let mut stmt = self.store.conn.prepare(&sql).map_err(|e| e.to_string())?;
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let files: Vec<(String, String)> = stmt
            .query_map(refs.as_slice(), |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();
        let mut readers: Vec<Value> = Vec::new();
        let mut scanned = 0usize;
        for (r, p) in files {
            scanned += 1;
            let abs = self.abs_of(&r, &p);
            let text = match std::fs::read_to_string(&abs) {
                Ok(t) => t,
                Err(_) => continue,
            };
            if !needles.iter().any(|n| text.contains(n.as_str())) {
                continue;
            }
            for (i, line) in text.lines().enumerate() {
                if !needles.iter().any(|n| line.contains(n.as_str())) {
                    continue;
                }
                readers.push(json!({
                    "root": r, "path": p, "line": i + 1, "text": line.trim(),
                }));
                if readers.len() as i64 >= limit.max(1) {
                    return Ok(json!({"tables": tables, "readers": readers, "scanned_files": scanned}));
                }
            }
        }
        Ok(json!({"tables": tables, "readers": readers, "scanned_files": scanned}))
    }
}
