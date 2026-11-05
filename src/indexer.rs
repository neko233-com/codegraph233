use crate::assets;
use crate::binder::Binder;
use crate::config::{is_excluded, Config};
use crate::model::FileParse;
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
        "lua" if f.code => Some("lua"),
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
        if matches!(lang, "csharp" | "go" | "rust" | "typescript" | "lua") && !cfg.language_enabled(lang) {
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
        "lua" => match std::fs::read(&f.abs) {
            Ok(data) => crate::lua_lang::parse(&f.rel, &f.root, &data),
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
        _ => {
            if assets::is_structured_asset(&f.rel) {
                assets::parse_structured_asset(&f.rel, &f.root, &f.abs)
            } else {
                assets::parse_media(&f.rel, &f.root, &f.abs)
            }
        }
    }
}

/// Parse one file with the project's feature set; used by `codegraph233 parse`
/// to debug what the indexer extracts.
pub fn parse_file_debug(abs: &Path, rel: &str, cfg: &Config) -> FileParse {
    let lang = lang_of(rel, cfg, &[]).unwrap_or("asset").to_string();
    let f = ScanFile {
        root: cfg
            .raw
            .roots
            .first()
            .map(|r| r.key.clone())
            .unwrap_or_else(|| "client".into()),
        abs: abs.to_path_buf(),
        rel: rel.to_string(),
        lang,
        size: std::fs::metadata(abs).map(|m| m.len() as i64).unwrap_or(0),
        mtime: 0,
    };
    parse_file(&f, cfg)
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

/// Bump when parsers change in a way that must invalidate existing rows.
const PARSER_VERSION: &str = "0.3.1";

pub fn run(cfg: &Config, opts: &Options) -> Result<Stats, String> {
    let started = Instant::now();
    let db_path = cfg.db_path();

    // An index written by an older schema (or by older parsers) cannot answer
    // the current queries. Indexes are disposable, so rebuild instead of
    // migrating in place.
    if db_path.is_file() {
        let (stale, rows) = match Store::open(&db_path, opts.fast) {
            Ok(probe) => (
                probe.is_stale().unwrap_or(false)
                    || probe.get_meta("parser_version") != PARSER_VERSION,
                probe.counts().0,
            ),
            Err(_) => (false, 0),
        };
        if stale && rows > 0 {
            for suffix in ["", "-wal", "-shm", "-journal"] {
                let p = PathBuf::from(format!("{}{suffix}", db_path.display()));
                let _ = std::fs::remove_file(p);
            }
            if !opts.quiet {
                eprintln!("[codegraph233] index schema/parser changed; rebuilding from scratch");
            }
        }
    }

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

    let prev_indexed = store.get_meta("indexed_at");
    let stale: Vec<i64> = states
        .iter()
        .filter(|(k, _)| !present.contains(*k))
        .map(|(_, st)| st.id)
        .collect();

    // Nothing changed: skip parsing *and* rebinding. This is the common case
    // for a watcher-driven index call and used to cost a full global rebind.
    if changed.is_empty() && stale.is_empty() && !opts.force && !prev_indexed.is_empty() {
        // The guid map may have been refreshed above; re-materialize so resolved
        // references reflect it even when no source file changed.
        if cfg.features.guid_map {
            let _ = store.materialize_guid_edges();
        }
        let (fcount, scount, rcount, ecount) = store.counts();
        let assets_count = store
            .conn
            .query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))
            .unwrap_or(0);
        let elapsed = started.elapsed().as_millis();
        if !opts.quiet {
            eprintln!("[codegraph233] no changes, {fcount} files, {elapsed}ms");
        }
        return Ok(Stats {
            files: fcount,
            changed: 0,
            removed: 0,
            symbols: scount,
            refs: rcount,
            edges: ecount,
            assets: assets_count,
            unresolved: 0,
            elapsed_ms: elapsed,
        });
    }

    let mut old_symbol_ids: Vec<i64> = Vec::new();
    let mut removed = 0usize;
    if !stale.is_empty() {
        let tx = store.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for id in &stale {
            old_symbol_ids.extend(Store::symbol_ids_of_file(&tx, *id)?);
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

    let mut changed_file_ids: Vec<i64> = Vec::new();
    let mut new_symbol_ids: Vec<i64> = Vec::new();
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
                old_symbol_ids.extend(Store::symbol_ids_of_file(&tx, id)?);
                Store::delete_file(&tx, id)?;
            }
            let file_id = Store::upsert_file(&tx, &f.root, &f.rel, &f.lang, &res.module, f.size, f.mtime)?;
            changed_file_ids.push(file_id);
            let ids = Store::insert_symbols(&tx, file_id, &f.root, &res.symbols)?;
            new_symbol_ids.extend(ids.iter().copied());
            let mut src_keys: HashMap<String, (i64, String)> = HashMap::with_capacity(res.symbols.len());
            for (i, s) in res.symbols.iter().enumerate() {
                src_keys.insert(s.key.clone(), (ids[i], s.namespace.clone()));
            }
            Store::insert_refs(&tx, file_id, &res.refs, &src_keys)?;
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

    // Binding. A small delta rebinds only refs that can have changed: those
    // from the changed files, those pointing at their symbols, and previously
    // unresolved refs whose name now exists. A large delta (or --force) keeps
    // the simpler global pass.
    let small_delta = changed.len() <= 2_000 && changed.len() * 3 <= files.len().max(10);
    let incremental = !opts.force
        && !prev_indexed.is_empty()
        && small_delta
        && old_symbol_ids.len() + new_symbol_ids.len() <= 60_000;

    let symbol_rows = store.symbol_rows()?;
    let binder = Binder::new(symbol_rows);

    let mut affected: Vec<i64> = Vec::new();
    let refs = if incremental {
        affected.extend(old_symbol_ids.iter().copied());
        affected.extend(new_symbol_ids.iter().copied());
        affected.sort_unstable();
        affected.dedup();
        let mut picked = store.ref_rows_for(&changed_file_ids, &affected)?;
        let mut seen: HashSet<i64> = picked.iter().map(|r| r.id).collect();
        let new_names: Vec<String> = new_symbol_ids
            .iter()
            .filter_map(|id| binder.row_by_id(*id).map(|s| s.name.clone()))
            .collect();
        for r in store.ref_rows_unresolved_named(&new_names)? {
            if seen.insert(r.id) {
                picked.push(r);
            }
        }
        let tx = store.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        Store::delete_edges_touching(&tx, &affected)?;
        tx.commit().map_err(|e| e.to_string())?;
        picked
    } else {
        store.ref_rows()?
    };

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
    if !incremental {
        Store::clear_edges(&tx)?;
    }
    let pre_unresolved = refs.iter().filter(|r| r.target == 0).count() as i64;
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
        match store.materialize_guid_edges() {
            Ok(n) => {
                if !opts.quiet {
                    eprintln!("[codegraph233] materialized {n} guid edges");
                }
            }
            Err(e) => eprintln!("[codegraph233] guid materialization failed: {e}"),
        }
    }

    store.set_meta("indexed_at", &crate::store::now_unix().to_string())?;
    store.set_meta("profile", &cfg.raw.profile)?;
    store.set_meta("parser_version", PARSER_VERSION)?;
    store.set_meta("rebind_mode", if incremental { "incremental" } else { "global" })?;
    let unresolved_total: i64 = if incremental {
        let prev: i64 = store.get_meta("unresolved_refs").parse().unwrap_or(-1);
        if prev >= 0 {
            (prev - pre_unresolved + unresolved as i64).max(0)
        } else {
            store
                .conn
                .query_row("SELECT COUNT(*) FROM refs WHERE COALESCE(target,0)=0", [], |r| r.get(0))
                .unwrap_or(0)
        }
    } else {
        unresolved as i64
    };
    store.set_meta("unresolved_refs", &unresolved_total.to_string())?;
    let (fcount, scount, rcount, ecount) = store.counts();
    let assets_count: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))
        .unwrap_or(0);
    let elapsed = started.elapsed().as_millis();

    if !opts.quiet {
        eprintln!(
            "[codegraph233] {fcount} files, {} changed, {removed} removed, {scount} symbols, {rcount} refs, {ecount} edges, {assets_count} assets, {unresolved} unresolved, rebind={}, {elapsed}ms",
            changed.len(),
            if incremental { "incremental" } else { "global" }
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
    pub stale: bool,
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
        stale: false,
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
        st.stale = store.is_stale().unwrap_or(false);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use std::fs;

    fn write(path: &Path, text: &str) {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    /// Edges keyed by qualified names so a rebuild (which renumbers ids) can be
    /// compared with an incremental pass.
    fn edge_set(db: &Path) -> Vec<(String, String, String)> {
        let store = Store::open(db, false).unwrap();
        let mut stmt = store
            .conn
            .prepare(
                "SELECT s.qualname, d.qualname, e.kind FROM edges e
                 JOIN symbols s ON s.id=e.src JOIN symbols d ON d.id=e.dst",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap();
        let mut out: Vec<(String, String, String)> = rows.flatten().collect();
        out.sort();
        out.dedup();
        out
    }

    fn temp_project(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cg233-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        write(
            &dir.join(".codegraph233.json"),
            r#"{"version":1,"profile":"code","roots":[{"key":"app","path":".","role":"frontend"}],"languages":["csharp"]}"#,
        );
        write(
            &dir.join("A.cs"),
            "namespace Game { public class A { public void Foo() {} public void Baz() {} } }",
        );
        write(
            &dir.join("B.cs"),
            "namespace Game { public class B { public void Bar() { var a = new A(); a.Foo(); } } }",
        );
        dir
    }

    fn opts() -> Options {
        Options { force: false, jobs: 2, quiet: true, fast: true }
    }

    #[test]
    fn incremental_rebind_matches_full_rebuild() {
        let dir = temp_project("incr");
        let cfg = config::load(&dir).unwrap();
        let first = run(&cfg, &opts()).unwrap();
        assert_eq!(first.changed, 2);
        let baseline = edge_set(&cfg.db_path());
        assert!(!baseline.is_empty(), "fixture should produce edges");

        // Touch one file: a trailing comment changes size/mtime only.
        write(
            &dir.join("B.cs"),
            "namespace Game { public class B { public void Bar() { var a = new A(); a.Foo(); a.Baz(); } } }\n// touched",
        );
        let inc = run(&cfg, &opts()).unwrap();
        assert_eq!(inc.changed, 1, "only B.cs changed");
        let store = Store::open(&cfg.db_path(), false).unwrap();
        assert_eq!(store.get_meta("rebind_mode"), "incremental");
        let incremental_edges = edge_set(&cfg.db_path());
        assert!(incremental_edges.iter().any(|(s, d, k)| s.ends_with("B.Bar") && d.ends_with("A.Baz") && k == "calls"));
        drop(store);

        let mut forced = opts();
        forced.force = true;
        run(&cfg, &forced).unwrap();
        let full = edge_set(&cfg.db_path());
        assert_eq!(incremental_edges, full, "incremental graph must equal a full rebuild");

        // A second call with no edits must not rebind or reparse.
        let idle = run(&cfg, &opts()).unwrap();
        assert_eq!(idle.changed, 0);
        assert_eq!(idle.removed, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn removal_prunes_symbols_and_edges() {
        let dir = temp_project("remove");
        let cfg = config::load(&dir).unwrap();
        run(&cfg, &opts()).unwrap();
        fs::remove_file(dir.join("B.cs")).unwrap();
        let st = run(&cfg, &opts()).unwrap();
        assert_eq!(st.removed, 1);
        let store = Store::open(&cfg.db_path(), false).unwrap();
        let left: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM symbols WHERE qualname LIKE '%B.Bar%'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
        drop(store);
        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod asset_edge_tests {
    use super::*;
    use crate::config;
    use std::fs;

    fn write(path: &Path, text: &str) {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn guid_materialization_links_material_to_shader_once() {
        let dir = std::env::temp_dir().join(format!("cg233-guid-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        write(
            &dir.join(".codegraph233.json"),
            r#"{"version":1,"profile":"full","roots":[{"key":"client","path":".","role":"frontend"}],"languages":["csharp"]}"#,
        );
        write(
            &dir.join("Assets/M.mat"),
            "%YAML 1.1\n--- !u!21 &2100000\nMaterial:\n  m_Name: M\n  m_Shader: {fileID: 4800000, guid: 11111111111111111111111111111111, type: 3}\n",
        );
        write(
            &dir.join("Assets/S.shader"),
            "Shader \"Game/S\" { Properties { _MainTex (\"T\", 2D) = \"white\" {} } SubShader { Pass { } } }\n",
        );
        // Plaintext .meta rows share the target's path; re-parsing them must not
        // drop references to the asset they describe.
        write(
            &dir.join("Assets/S.shader.meta"),
            "fileFormatVersion: 2\nguid: 11111111111111111111111111111111\nShaderImporter:\n  defaultTextures: []\n",
        );
        write(
            &dir.join(".codegraph233/guid-map.tsv"),
            "guid\tpath\ttype\n11111111111111111111111111111111\tAssets/S.shader\tShader\n",
        );
        let cfg = config::load(&dir).unwrap();
        let opts = Options { force: false, jobs: 2, quiet: true, fast: true };
        run(&cfg, &opts).unwrap();
        run(&cfg, &opts).unwrap();
        let store = Store::open(&cfg.db_path(), false).unwrap();
        let rows: Vec<(String, String)> = store
            .conn
            .prepare(
                "SELECT dst, kind FROM asset_edges WHERE kind='uses_shader' ORDER BY dst",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(rows.len(), 1, "expected exactly one materialized shader edge: {rows:?}");
        assert!(rows[0].0.ends_with("Assets/S.shader"), "{rows:?}");
        drop(store);
        let _ = fs::remove_dir_all(&dir);
    }
}
