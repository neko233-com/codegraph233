use crate::assets;
use crate::binder::Binder;
use crate::config::{is_excluded, Config};
use crate::model::{symbol_key, FileParse};
use crate::store::Store;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Default, Clone)]
pub struct Options {
    pub force: bool,
    pub jobs: usize,
    pub quiet: bool,
    pub fast: bool,
}

#[derive(Debug, Clone)]
pub struct Stats {
    pub files: i64,
    pub changed: usize,
    pub removed: usize,
    pub symbols: i64,
    pub refs: i64,
    pub edges: i64,
    pub assets: i64,
    pub unresolved: usize,
    pub elapsed_ms: u128,
}

#[derive(Clone)]
struct ScanFile {
    root: String,
    abs: PathBuf,
    rel: String,
    lang: String,
    size: i64,
    mtime: i64,
}

fn lang_of(rel: &str, cfg: &Config, config_dirs: &[PathBuf]) -> Option<&'static str> {
    let ext = Path::new(rel)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let f = &cfg.features;
    match ext.as_str() {
        "cs" if f.code => Some("csharp"),
        "go" if f.code => Some("go"),
        "rs" if f.code => Some("rust"),
        "ts" | "tsx" | "js" | "jsx" if f.code => Some("typescript"),
        "prefab" | "unity" if f.prefabs => Some("prefab"),
        "shader" | "hlsl" | "cginc" | "compute" if f.shaders => Some("shader"),
        "xlsx" | "csv" | "tsv" if f.config_tables => Some("config"),
        "json" if f.config_tables => {
            let abs = Path::new(rel);
            let under = config_dirs.iter().any(|d| abs.starts_with(d));
            if under { Some("config") } else { None }
        }
        "meta" => {
            if f.meta {
                Some("meta")
            } else {
                None
            }
        }
        _ => {
            if f.assets && assets::is_asset_ext(rel) {
                Some("asset")
            } else {
                None
            }
        }
    }
}

fn walk(dir: &Path, base: &Path, root_key: &str, excludes: &[String], cfg: &Config, config_dirs: &[PathBuf], out: &mut Vec<ScanFile>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let rel = path
            .strip_prefix(base)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if path.is_dir() {
            if is_excluded(&rel, &name, excludes) {
                continue;
            }
            walk(&path, base, root_key, excludes, cfg, config_dirs, out);
            continue;
        }
        let Some(lang) = lang_of(&rel, cfg, config_dirs) else { continue };
        if matches!(lang, "csharp" | "go" | "rust" | "typescript") && !cfg.language_enabled(lang) {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            out.push(ScanFile {
                root: root_key.to_string(),
                abs: path,
                rel,
                lang: lang.to_string(),
                size: meta.len() as i64,
                mtime: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as i64)
                    .unwrap_or(0),
            });
        }
    }
}

fn collect(cfg: &Config) -> Vec<ScanFile> {
    let excludes = cfg.excludes();
    let config_dirs: Vec<PathBuf> = cfg
        .raw
        .config_dirs
        .iter()
        .map(|d| {
            let p = Path::new(d);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                cfg.dir.join(p)
            }
        })
        .collect();
    let mut out = Vec::new();
    for r in &cfg.raw.roots {
        let dir = cfg.abs_root(r);
        if !dir.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        walk(&dir, &dir, &r.key, &excludes, cfg, &config_dirs, &mut files);
        out.extend(files);
    }
    out
}

fn go_module_of(abs: &Path) -> String {
    let mut dir = abs.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for _ in 0..8 {
        if let Ok(text) = std::fs::read_to_string(dir.join("go.mod")) {
            for line in text.lines() {
                if let Some(v) = line.trim().strip_prefix("module ") {
                    return v.trim().to_string();
                }
            }
            return String::new();
        }
        if !dir.pop() {
            break;
        }
    }
    String::new()
}

fn parse_file(f: &ScanFile, cfg: &Config) -> FileParse {
    match f.lang.as_str() {
        "csharp" => match std::fs::read(&f.abs) {
            Ok(data) => crate::csharp::parse(&f.rel, &f.root, &data, cfg.features.type_deps),
            Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
        },
        "go" => match std::fs::read(&f.abs) {
            Ok(data) => {
                let module = go_module_of(&f.abs);
                let rel_dir = Path::new(&f.rel)
                    .parent()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                crate::go_lang::parse(&f.rel, &f.root, &module, &rel_dir, &data)
            }
            Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
        },
        "rust" => match std::fs::read(&f.abs) {
            Ok(data) => crate::rust_lang::parse(&f.rel, &f.root, &data),
            Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
        },
        "typescript" => match std::fs::read(&f.abs) {
            Ok(data) => crate::ts_lang::parse(&f.rel, &f.root, &data),
            Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
        },
        "prefab" => match std::fs::read_to_string(&f.abs) {
            Ok(text) => assets::parse_prefab(&f.rel, &f.root, &text),
            Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
        },
        "shader" => match std::fs::read_to_string(&f.abs) {
            Ok(text) => assets::parse_shader(&f.rel, &f.root, &text),
            Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
        },
        "config" => assets::parse_config(&f.rel, &f.root, &f.abs),
        "meta" => match std::fs::read_to_string(&f.abs) {
            Ok(text) => assets::parse_meta(&f.rel, &f.root, &text),
            Err(_) => FileParse::default(),
        },
        _ => assets::parse_media(&f.rel, &f.root, &f.abs),
    }
}

fn load_guid_maps(cfg: &Config) -> Vec<crate::model::GuidRow> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for g in &cfg.raw.guid_map {
        let p = Path::new(g);
        candidates.push(if p.is_absolute() { p.to_path_buf() } else { cfg.dir.join(p) });
    }
    candidates.push(cfg.dir.join(".codegraph233").join("guid-map.tsv"));
    candidates.push(cfg.dir.join("Temp").join("codegraph233-guid-map.tsv"));
    let mut out = Vec::new();
    for c in candidates {
        if c.is_file() {
            if let Ok(rows) = assets::parse_guid_map_file(&c) {
                out.extend(rows);
            }
        }
    }
    out
}

pub fn run(cfg: &Config, opts: &Options) -> Result<Stats, String> {
    let started = Instant::now();
    let db_path = cfg.db_path();
    let store = Store::open(&db_path, opts.fast)?;

    if cfg.features.guid_map {
        let rows = load_guid_maps(cfg);
        if !rows.is_empty() {
            store.replace_guid_map(&rows)?;
        }
    }

    let files = collect(cfg);
    let mut present: HashSet<String> = HashSet::new();
    for f in &files {
        present.insert(format!("{}|{}", f.root, f.rel));
    }
    let states = store.file_states()?;
    let changed: Vec<ScanFile> = files
        .iter()
        .filter(|f| {
            let key = format!("{}|{}", f.root, f.rel);
            match states.get(&key) {
                None => true,
                Some(st) => opts.force || st.size != f.size || st.mtime != f.mtime || st.lang != f.lang,
            }
        })
        .cloned()
        .collect();

    let mut removed = 0usize;
    let stale: Vec<i64> = states
        .iter()
        .filter(|(k, _)| !present.contains(*k))
        .map(|(_, st)| st.id)
        .collect();
    if !stale.is_empty() {
        let tx = store.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for id in &stale {
            Store::delete_file(&tx, *id)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        removed = stale.len();
    }

    let jobs = if opts.jobs > 0 {
        opts.jobs
    } else {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .map_err(|e| e.to_string())?;
    let parsed: Vec<FileParse> = pool.install(|| {
        changed
            .par_iter()
            .map(|f| parse_file(f, cfg))
            .collect()
    });

    const BATCH: usize = 800;
    let mut idx = 0usize;
    while idx < changed.len() {
        let end = (idx + BATCH).min(changed.len());
        let tx = store.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for k in idx..end {
            let f = &changed[k];
            let res = &parsed[k];
            let old_id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM files WHERE root=?1 AND path=?2",
                    rusqlite::params![f.root, f.rel],
                    |r| r.get(0),
                )
                .ok();
            if let Some(id) = old_id {
                Store::delete_file(&tx, id)?;
            }
            let file_id = Store::upsert_file(&tx, &f.root, &f.rel, &f.lang, &res.module, f.size, f.mtime)?;
            let ids = Store::insert_symbols(&tx, file_id, &res.symbols)?;
            let mut key_to_id: HashMap<String, i64> = HashMap::with_capacity(res.symbols.len());
            for (i, s) in res.symbols.iter().enumerate() {
                key_to_id.insert(symbol_key(&s.qualname, &s.kind, s.start_line), ids[i]);
            }
            Store::insert_refs(&tx, file_id, &res.refs, &key_to_id)?;
            Store::insert_assets_and_edges(&tx, &res.assets, &res.asset_edges)?;
            if !res.guid_rows.is_empty() {
                let rows: Vec<crate::model::GuidRow> = res.guid_rows.clone();
                // plaintext meta mappings only fill gaps
                let mut stmt = tx
                    .prepare("INSERT OR IGNORE INTO guid_map(guid, path, name, type) VALUES(?1,?2,?3,?4)")
                    .map_err(|e| e.to_string())?;
                for r in rows {
                    let name = Path::new(&r.path)
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let _ = stmt.execute(rusqlite::params![r.guid, r.path, name, r.kind]);
                }
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        idx = end;
    }

    // binding
    let symbol_rows = store.symbol_rows()?;
    let binder = Binder::new(symbol_rows);
    let refs = store.ref_rows()?;
    let resolved: Vec<Option<(i64, &'static str)>> = pool.install(|| {
        refs.par_iter()
            .map(|r| {
                if r.src_id == 0 {
                    return None;
                }
                let src = binder.row_by_id(r.src_id)?;
                binder.resolve(r, src).map(|res| (binder.row(res.row).id, res.confidence))
            })
            .collect()
    });

    let tx = store.conn.unchecked_transaction().map_err(|e| e.to_string())?;
    Store::clear_edges(&tx)?;
    let mut updates: Vec<(i64, i64, String)> = Vec::new();
    let mut edges: Vec<(i64, i64, String, i32, String)> = Vec::new();
    let mut seen: HashSet<(i64, i64, String)> = HashSet::new();
    let mut unresolved = 0usize;
    for (i, r) in refs.iter().enumerate() {
        match resolved[i] {
            None => {
                unresolved += 1;
                if r.target != 0 {
                    updates.push((r.id, 0, "unresolved".into()));
                }
            }
            Some((target, conf)) => {
                if r.target != target {
                    updates.push((r.id, target, conf.into()));
                }
                if target != r.src_id {
                    let k = (r.src_id, target, r.kind.clone());
                    if seen.insert(k) {
                        edges.push((r.src_id, target, r.kind.clone(), r.line, conf.into()));
                    }
                }
            }
        }
    }
    Store::update_targets(&tx, &updates)?;
    Store::insert_edges(&tx, &edges)?;
    tx.commit().map_err(|e| e.to_string())?;

    if cfg.features.guid_map {
        let _ = store.materialize_guid_edges();
    }

    store.set_meta("indexed_at", &crate::store::now_unix().to_string())?;
    store.set_meta("profile", &cfg.raw.profile)?;
    let (fcount, scount, rcount, ecount) = store.counts();
    let assets_count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))
        .unwrap_or(0);
    let elapsed = started.elapsed().as_millis();

    if !opts.quiet {
        eprintln!(
            "[codegraph233] {fcount} files, {} changed, {removed} removed, {scount} symbols, {rcount} refs, {ecount} edges, {assets_count} assets, {unresolved} unresolved, {elapsed}ms",
            changed.len()
        );
    }

    Ok(Stats {
        files: fcount,
        changed: changed.len(),
        removed,
        symbols: scount,
        refs: rcount,
        edges: ecount,
        assets: assets_count,
        unresolved,
        elapsed_ms: elapsed,
    })
}

pub struct Status {
    pub exists: bool,
    pub db: String,
    pub files: i64,
    pub symbols: i64,
    pub refs: i64,
    pub edges: i64,
    pub assets: i64,
    pub guid_entries: i64,
    pub indexed_at: i64,
}

pub fn status(cfg: &Config) -> Status {
    let db = cfg.db_path();
    let exists = db.is_file();
    let mut st = Status {
        exists,
        db: db.display().to_string(),
        files: 0,
        symbols: 0,
        refs: 0,
        edges: 0,
        assets: 0,
        guid_entries: 0,
        indexed_at: 0,
    };
    if !exists {
        return st;
    }
    if let Ok(store) = Store::open(&db, false) {
        let (f, s, r, e) = store.counts();
        st.files = f;
        st.symbols = s;
        st.refs = r;
        st.edges = e;
        st.assets = store
            .conn
            .query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))
            .unwrap_or(0);
        st.guid_entries = store.guid_count();
        st.indexed_at = store.get_meta("indexed_at").parse().unwrap_or(0);
    }
    st
}
