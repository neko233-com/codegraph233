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

fn walk(dir: &Path, base: &Path, root_key: &str, excludes: &[String], out: &mut Vec<ScanFile>) {
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
            walk(&path, base, root_key, excludes, out);
        } else {
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let lang = match ext.as_str() {
                "cs" => "csharp",
                "go" => "go",
                "rs" => "rust",
                "ts" | "tsx" | "js" | "jsx" => "typescript",
                _ => continue,
            };
            if let Ok(meta) = entry.metadata() {
                out.push(ScanFile {
                    root: root_key.to_string(),
                    abs: path,
                    rel: String::new(),
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
}

fn collect(cfg: &Config) -> Vec<ScanFile> {
    let excludes = cfg.excludes();
    let mut out = Vec::new();
    for r in &cfg.raw.roots {
        let dir = cfg.abs_root(r);
        if !dir.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        walk(&dir, &dir, &r.key, &excludes, &mut files);
        for mut f in files {
            if let Ok(rel) = f.abs.strip_prefix(&dir) {
                f.rel = rel.to_string_lossy().replace('\\', "/");
            }
            if !cfg.language_enabled(&f.lang) {
                continue;
            }
            out.push(f);
        }
    }
    out
}

pub fn run(cfg: &Config, opts: &Options) -> Result<Stats, String> {
    let started = Instant::now();
    let db_path = cfg.db_path();
    let mut store = Store::open(&db_path, opts.fast)?;

    let files = collect(cfg);
    let mut present: HashSet<String> = HashSet::new();
    for f in &files {
        present.insert(format!("{}|{}", f.root, f.rel));
    }

    let states = store.file_states()?;
    let mut changed: Vec<ScanFile> = Vec::new();
    for f in files.iter() {
        let key = format!("{}|{}", f.root, f.rel);
        let need = match states.get(&key) {
            None => true,
            Some(st) => opts.force || st.size != f.size || st.mtime != f.mtime || st.lang != f.lang,
        };
        if need && f.lang == "csharp" {
            changed.push(f.clone());
        }
    }

    // prune rows no longer present
    let mut removed = 0usize;
    let stale: Vec<i64> = states
        .iter()
        .filter(|(k, _)| !present.contains(*k))
        .map(|(_, st)| st.id)
        .collect();
    if !stale.is_empty() {
        let tx = store.conn.transaction().map_err(|e| e.to_string())?;
        for id in &stale {
            Store::delete_file(&tx, *id)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        removed = stale.len();
    }

    // parse (parallel)
    let jobs = if opts.jobs > 0 {
        opts.jobs
    } else {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .map_err(|e| e.to_string())?;
    let type_deps = cfg.features.type_deps;
    let parsed: Vec<FileParse> = pool.install(|| {
        changed
            .par_iter()
            .map(|f| match std::fs::read(&f.abs) {
                Ok(data) => crate::csharp::parse(&f.rel, &f.root, &data, type_deps),
                Err(e) => FileParse {
                    rel_path: f.rel.clone(),
                    root: f.root.clone(),
                    lang: f.lang.clone(),
                    module: String::new(),
                    symbols: Vec::new(),
                    refs: Vec::new(),
                    errors: vec![e.to_string()],
                },
            })
            .collect()
    });

    // write in batches
    const BATCH: usize = 1000;
    let mut idx = 0usize;
    while idx < changed.len() {
        let end = (idx + BATCH).min(changed.len());
        let tx = store.conn.transaction().map_err(|e| e.to_string())?;
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
            let file_id = Store::upsert_file(
                &tx, &f.root, &f.rel, &f.lang, &res.module, f.size, f.mtime,
            )?;
            let ids = Store::insert_symbols(&tx, file_id, &res.symbols)?;
            let mut key_to_id: HashMap<String, i64> = HashMap::with_capacity(res.symbols.len());
            for (i, s) in res.symbols.iter().enumerate() {
                key_to_id.insert(symbol_key(&s.qualname, &s.kind, s.start_line), ids[i]);
            }
            Store::insert_refs(&tx, file_id, &res.refs, &key_to_id)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        idx = end;
    }

    // rebind
    let symbol_rows = store.symbol_rows()?;
    let binder = Binder::new(symbol_rows);
    let mut refs = store.ref_rows()?;
    let resolved: Vec<Option<(i64, &'static str)>> = pool.install(|| {
        refs.par_iter()
            .map(|r| {
                if r.src_id == 0 {
                    return None;
                }
                let src = binder.row_by_id(r.src_id)?;
                binder.resolve(r, src).map(|res| {
                    let row = binder.row(res.row);
                    (row.id, res.confidence)
                })
            })
            .collect()
    });

    let tx = store.conn.transaction().map_err(|e| e.to_string())?;
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
                    if seen.insert(k.clone()) {
                        edges.push((r.src_id, target, r.kind.clone(), r.line, conf.into()));
                    }
                }
            }
        }
    }
    Store::update_targets(&tx, &updates)?;
    Store::insert_edges(&tx, &edges)?;
    tx.commit().map_err(|e| e.to_string())?;
    refs.clear();

    store.set_meta("indexed_at", &crate::store::now_unix().to_string())?;
    store.set_meta("profile", &cfg.raw.profile)?;
    let (fcount, scount, rcount, ecount) = store.counts();
    let elapsed = started.elapsed().as_millis();

    if !opts.quiet {
        eprintln!(
            "[codegraph233] {} files, {} changed, {} removed, {} symbols, {} refs, {} edges, {} unresolved, {}ms",
            fcount, changed.len(), removed, scount, rcount, ecount, unresolved, elapsed
        );
    }

    Ok(Stats {
        files: fcount,
        changed: changed.len(),
        removed,
        symbols: scount,
        refs: rcount,
        edges: ecount,
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
        st.indexed_at = store.get_meta("indexed_at").parse().unwrap_or(0);
    }
    st
}
