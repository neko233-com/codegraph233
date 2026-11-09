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
    pub parse_errors: usize,
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

fn is_repo_doc(rel: &str) -> bool {
    let name = rel
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(rel)
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        "readme.md"
            | "changelog.md"
            | "license.md"
            | "contributing.md"
            | "security.md"
            | "code_of_conduct.md"
            | "agents.md"
    )
}

fn lang_of(rel: &str, cfg: &Config, config_dirs: &[PathBuf]) -> Option<&'static str> {
    let ext = Path::new(rel)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let f = &cfg.features;
    if let Some(target) = cfg.extra_extensions.get(&ext) {
        if let Some(spec) = crate::langs::spec(target) {
            return Some(spec.id);
        }
    }
    match ext.as_str() {
        "cs" if f.code => Some("csharp"),
        "go" if f.code => Some("go"),
        "rs" if f.code => Some("rust"),
        "java" if f.code => Some("java"),
        "class" | "jar" if f.code => Some("jvm"),
        "c" if f.code => Some("c"),
        "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hh" | "hxx" if f.code => Some("cpp"),
        "h" if f.code => Some(if cfg.language_enabled("cpp") { "cpp" } else { "c" }),
        "py" | "pyi" if f.code => Some("python"),
        "ps1" | "psm1" | "psd1" if f.code => Some("powershell"),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" if f.code => Some("typescript"),
        "jsonl" if f.code && rel.ends_with(".cg-jvm.jsonl") => Some("jvm"),
        "lua" if f.code => Some("lua"),
        "luau" if f.code => Some("lua"),
        "gd" if f.code => Some("gdscript"),
        "kt" | "kts" if f.code => Some("kotlin"),
        "swift" if f.code => Some("swift"),
        "m" | "mm" if f.code => Some("objc"),
        "dart" if f.code => Some("dart"),
        "hx" if f.code => Some("haxe"),
        "gml" if f.code => Some("gml"),
        "prefab" | "unity" if f.prefabs => Some("prefab"),
        "shader" if f.shaders => Some("shader"),
        "hlsl" | "cginc" | "compute" | "usf" | "ush" | "cg" if f.code || f.shaders => Some("hlsl"),
        "glsl" | "vert" | "frag" | "geom" | "comp" | "tesc" | "tese" | "gdshader" if f.code || f.shaders => Some("glsl"),
        "metal" if f.code || f.shaders => Some("metal"),
        "wgsl" if f.code || f.shaders => Some("wgsl"),
        "uproject" if f.assets || f.code => Some("ue_project"),
        "uplugin" if f.assets || f.code => Some("ue_plugin"),
        "ini" if f.assets || f.config_tables => Some("ue_config"),
        "uasset" | "umap" if f.assets => Some("ue_asset"),
        "tscn" | "tres" if f.assets => Some("godot_scene"),
        "godot" if f.assets || f.config_tables => Some("godot_project"),
        "mixer" if f.assets => Some("unity_mixer"),
        "wwu" if f.assets => Some("wwise_workunit"),
        "wproj" if f.assets => Some("wwise_project"),
        "bnk" if f.assets => Some("wwise_bank"),
        "wem" if f.assets => Some("wwise_media"),
        "fspro" if f.assets => Some("fmod_project"),
        "strings" if f.assets => Some("fmod_strings"),
        "bank" if f.assets => Some("fmod_bank"),
        "gltf" | "glb" if f.assets => Some("gltf"),
        "ase" | "aseprite" if f.assets => Some("aseprite"),
        "fbx" if f.assets => Some("fbx"),
        "json"
            if f.assets
                && (rel.to_ascii_lowercase().ends_with("packages/manifest.json")
                    || rel.to_ascii_lowercase().ends_with("packages/packages-lock.json")) =>
        {
            Some("upm_manifest")
        }
        "md" | "markdown" if f.assets && !is_repo_doc(rel) => Some("design_doc"),
        "yarn" if f.assets => Some("yarn_script"),
        "ink" if f.assets => Some("ink_script"),
        "srt" | "vtt" if f.assets => Some("subtitle"),
        "webm" | "mkv" if f.assets => Some("video"),
        "po" | "pot" if f.assets => Some("localization_po"),
        "xliff" | "xlf" if f.assets => Some("localization_xliff"),
        "resx" if f.assets => Some("localization_resx"),
        "arb" if f.assets => Some("localization_arb"),
        "csv" if (f.assets || f.config_tables) && rel.to_ascii_lowercase().contains("localization") => {
            Some("localization_csv")
        }
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
        if crate::config::is_code_lang(lang) && !cfg.language_enabled(lang) {
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
    let mut fp = parse_file_inner(f, cfg);
    if crate::config::is_code_lang(&f.lang) && f.lang != "jvm" {
        if let Ok(data) = std::fs::read(&f.abs) {
            if let Ok(text) = std::str::from_utf8(&data) {
                if text.contains("@design:") || text.contains("@spec:") || text.contains("@doc:") {
                    fp.asset_edges
                        .extend(crate::design_docs::code_markers(&f.rel, &f.root, text));
                }
                fp.asset_edges
                    .extend(crate::audio_refs::scan(&f.rel, &f.root, text));
            }
        }
    }
    fp
}

fn parse_file_inner(f: &ScanFile, cfg: &Config) -> FileParse {
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
        "jvm" => match std::fs::read(&f.abs) {
            Ok(data) => if f.rel.ends_with(".cg-jvm.jsonl") { crate::jvm_facts::parse(&f.rel, &f.root, &data) }
                else { crate::jvm_binary::parse(&f.rel, &f.root, &data, cfg.raw.jvm_release) },
            Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
        },
        "java" | "c" | "cpp" | "python" | "powershell" => match std::fs::read(&f.abs) {
            Ok(data) => crate::source_ast::parse_with_annotations(&f.rel, &f.root, &f.lang, &data, &cfg.raw.c_annotations),
            Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
        },
        "gdscript" | "kotlin" | "swift" | "objc" | "dart" => match std::fs::read(&f.abs) {
            Ok(data) => crate::tree_langs::parse(&f.rel, &f.root, &f.lang, &data),
            Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
        },
        "haxe" | "gml" | "hlsl" | "glsl" | "metal" | "wgsl" => match std::fs::read(&f.abs) {
            Ok(data) => crate::lexer_langs::parse(&f.rel, &f.root, &f.lang, &data),
            Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
        },
        "ue_project" | "ue_plugin" | "ue_config" | "godot_scene" | "godot_project" => {
            match std::fs::read_to_string(&f.abs) {
                Ok(text) => crate::engine_assets::parse_text(&f.rel, &f.root, &f.lang, &text),
                Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
            }
        }
        "ue_asset" => match std::fs::read(&f.abs) {
            Ok(data) => crate::engine_assets::parse_uasset(&f.rel, &f.root, &data),
            Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
        },
        "design_doc" | "yarn_script" | "ink_script" | "subtitle" | "video" => match std::fs::read(&f.abs) {
            Ok(data) => crate::design_docs::parse(&f.rel, &f.root, &f.lang, &data),
            Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
        },
        "localization_po" | "localization_xliff" | "localization_resx" | "localization_arb"
        | "localization_csv" => match std::fs::read(&f.abs) {
            Ok(data) => crate::localization::parse(&f.rel, &f.root, &f.lang, &data),
            Err(e) => FileParse { rel_path: f.rel.clone(), root: f.root.clone(), lang: f.lang.clone(), errors: vec![e.to_string()], ..Default::default() },
        },
        "unity_mixer" | "wwise_workunit" | "wwise_project" | "wwise_bank" | "wwise_media"
        | "fmod_project" | "fmod_strings" | "fmod_bank" | "gltf" | "aseprite" | "fbx"
        | "upm_manifest" => match std::fs::read(&f.abs) {
            Ok(data) => crate::pipeline_assets::parse(&f.rel, &f.root, &f.lang, &data),
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
const PARSER_VERSION: &str = "0.4.1-full10";

pub fn run(cfg: &Config, opts: &Options) -> Result<Stats, String> {
    let started = Instant::now();
    let db_path = cfg.db_path();
    let config_identity = serde_json::to_string(&cfg.raw).map_err(|e| e.to_string())?;

    // An index written by an older schema (or by older parsers) cannot answer
    // the current queries. Indexes are disposable, so rebuild instead of
    // migrating in place.
    if db_path.is_file() {
        let (stale, rows) = match Store::open(&db_path, opts.fast) {
            Ok(probe) => (
                probe.is_stale().unwrap_or(false)
                    || probe.get_meta("parser_version") != PARSER_VERSION
                    || probe.get_meta("config_identity") != config_identity,
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
            unresolved: store.get_meta("unresolved_refs").parse().unwrap_or(0),
            parse_errors: store.get_meta("parse_errors").parse().unwrap_or(0),
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
    } else if let Ok(jobs) = std::env::var("CODEGRAPH233_JOBS") {
        jobs.parse::<usize>().unwrap_or(1).clamp(1,64)
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
    for (file, result) in changed.iter().zip(&parsed) {
        if file.lang == "jvm" && !result.errors.is_empty() {
            return Err(format!("Invalid JVM facts {}: {}", file.rel, result.errors.join("; ")));
        }
    }

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
            let diagnostic_key = format!("diagnostic:{}|{}", f.root, f.rel);
            if res.errors.is_empty() {
                tx.execute("DELETE FROM meta WHERE key=?1", [&diagnostic_key]).map_err(|e| e.to_string())?;
            } else {
                let value = serde_json::json!({"root":f.root,"path":f.rel,"lang":f.lang,"errors":res.errors}).to_string();
                tx.execute("INSERT OR REPLACE INTO meta(key,value) VALUES(?1,?2)", rusqlite::params![diagnostic_key,value]).map_err(|e| e.to_string())?;
            }
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
            .filter_map(|id| binder.row_by_id(*id))
            .flat_map(|s| {
                let mut names = vec![s.name.clone()];
                if s.lang == "jvm" { names.push(s.qualname.clone()); }
                names
            })
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
    store.set_meta("config_identity", &config_identity)?;
    let mut parse_errors = 0usize;
    let diagnostics: Vec<(String,String)> = store.conn.prepare("SELECT key,value FROM meta WHERE key LIKE 'diagnostic:%'").map_err(|e| e.to_string())?
        .query_map([], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|e| e.to_string())?
        .collect::<Result<_,_>>().map_err(|e| e.to_string())?;
    for (key,value) in diagnostics {
        if !present.contains(key.trim_start_matches("diagnostic:")) {
            store.conn.execute("DELETE FROM meta WHERE key=?1", [&key]).map_err(|e| e.to_string())?;
        } else if let Ok(value) = serde_json::from_str::<serde_json::Value>(&value) {
            parse_errors += value["errors"].as_array().map(Vec::len).unwrap_or(0);
        }
    }
    store.set_meta("parse_errors", &parse_errors.to_string())?;
    store.set_meta("rebind_mode", if incremental { "incremental" } else { "global" })?;
    let unresolved_total: i64 = store.conn
        .query_row("SELECT COUNT(*) FROM refs WHERE COALESCE(target,0)=0", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
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
        unresolved: unresolved_total as usize,
        parse_errors,
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
    pub parse_errors: usize,
    pub parser_version: String,
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
        parse_errors: 0,
        parser_version: String::new(),
    };
    if !exists {
        return st;
    }
    if let Ok(store) = Store::open(&db, false) {
        st.parser_version = store.get_meta("parser_version");
        st.parse_errors = store.get_meta("parse_errors").parse().unwrap_or(0);
        st.stale = store.is_stale().unwrap_or(false) || st.parser_version != PARSER_VERSION
            || serde_json::to_string(&cfg.raw).map(|v| v != store.get_meta("config_identity")).unwrap_or(true);
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

    fn write_bytes(path: &Path, bytes: &[u8]) {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(path, bytes).unwrap();
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
    fn indexes_game_development_language_frontends() {
        let dir = std::env::temp_dir().join(format!("cg233-game-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        write(
            &dir.join(".codegraph233.json"),
            r#"{"version":1,"profile":"code","roots":[{"key":"app","path":".","role":"frontend"}],"languages":["auto"]}"#,
        );
        write(
            &dir.join("game/PlayerGd.gd"),
            "class_name PlayerGd\nextends Node2D\n\nvar health: int = 100\n\nfunc take_damage(amount: int) -> void:\n\thealth -= amount\n\nfunc _ready() -> void:\n\ttake_damage(10)\n",
        );
        write(
            &dir.join("game/Enemy.kt"),
            "package demo.game\n\nclass Enemy {\n    fun hit() {\n        takeDamage(5)\n    }\n\n    fun takeDamage(amount: Int) {\n        damage += amount\n    }\n}\n",
        );
        write(
            &dir.join("game/Hero.swift"),
            "class HeroSwift {\n    var hp: Int = 100\n    func takeDamage(_ amount: Int) {\n        hp -= amount\n    }\n    func update() {\n        takeDamage(3)\n    }\n}\n",
        );
        write(
            &dir.join("game/Bag.m"),
            "#import <Foundation/Foundation.h>\n@interface Bag : NSObject\n- (void)addItem:(NSString *)item withCount:(NSInteger)count;\n@end\n@implementation Bag\n- (void)addItem:(NSString *)item withCount:(NSInteger)count {\n    [self logItem:item count:count];\n}\n- (void)logItem:(NSString *)item count:(NSInteger)count {\n    NSLog(@\"%@\", item);\n}\n@end\n",
        );
        write(
            &dir.join("game/player.dart"),
            "class DartPlayer {\n  int hp = 100;\n  void takeDamage(int amount) {\n    hp -= amount;\n  }\n  void update() {\n    takeDamage(1);\n  }\n}\n",
        );
        write(
            &dir.join("game/HaxePlayer.hx"),
            "package demo;\n\nclass HaxePlayer {\n    public function new() {}\n    public function takeDamage(amount:Int):Void {\n        log(\"hit\");\n    }\n    function log(msg:String):Void {}\n}\n",
        );
        write(
            &dir.join("scripts/PlayerGml.gml"),
            "function PlayerGml(_name) constructor {\n    name = _name;\n    take_damage = function(amount) {\n        log_hit(\"hit\");\n    };\n    static log_hit = function(msg) {\n        show_debug_message(msg);\n    };\n}\n",
        );
        write(
            &dir.join("shaders/Phong.hlsl"),
            "float lighting(float3 n) {\n    return dot(n, n);\n}\n\nfloat4 main(float3 n) : SV_Target {\n    return float4(lighting(n), 1.0);\n}\n",
        );

        let cfg = config::load(&dir).unwrap();
        let stats = run(&cfg, &opts()).unwrap();
        assert_eq!(stats.parse_errors, 0, "frontends must parse the fixtures cleanly");
        let edges = edge_set(&cfg.db_path());
        let expect = |src: &str, dst: &str, kind: &str| {
            assert!(
                edges.contains(&(src.to_string(), dst.to_string(), kind.to_string())),
                "missing edge {src} -[{kind}]-> {dst}\n{edges:#?}"
            );
        };
        expect("PlayerGd._ready", "PlayerGd.take_damage", "calls");
        expect("demo.game.Enemy.hit", "demo.game.Enemy.takeDamage", "calls");
        expect("HeroSwift.update", "HeroSwift.takeDamage", "calls");
        expect("Bag.addItem:withCount:", "Bag.logItem:count:", "calls");
        expect("DartPlayer.update", "DartPlayer.takeDamage", "calls");
        expect("demo.HaxePlayer.takeDamage", "demo.HaxePlayer.log", "calls");
        expect("PlayerGml.take_damage", "PlayerGml.log_hit", "calls");
        expect("main", "lighting", "calls");
    }

    #[test]
    fn game_extensions_map_to_their_frontends() {
        let dir = temp_project("langmap");
        let cfg = config::load(&dir).unwrap();
        let dirs: Vec<PathBuf> = Vec::new();
        let map = |rel: &str| lang_of(rel, &cfg, &dirs);
        assert_eq!(map("Player.gd"), Some("gdscript"));
        assert_eq!(map("Player.kt"), Some("kotlin"));
        assert_eq!(map("Player.kts"), Some("kotlin"));
        assert_eq!(map("Player.swift"), Some("swift"));
        assert_eq!(map("Player.m"), Some("objc"));
        assert_eq!(map("Player.mm"), Some("objc"));
        assert_eq!(map("player.dart"), Some("dart"));
        assert_eq!(map("Player.hx"), Some("haxe"));
        assert_eq!(map("Player.gml"), Some("gml"));
        assert_eq!(map("Player.luau"), Some("lua"));
        assert_eq!(map("Shaders/Post.hlsl"), Some("hlsl"));
        assert_eq!(map("Shaders/Post.usf"), Some("hlsl"));
        assert_eq!(map("Shaders/Common.ush"), Some("hlsl"));
        assert_eq!(map("Shaders/Legacy.cg"), Some("hlsl"));
        assert_eq!(map("Shaders/Post.glsl"), Some("glsl"));
        assert_eq!(map("Shaders/Post.gdshader"), Some("glsl"));
        assert_eq!(map("Shaders/Post.metal"), Some("metal"));
        assert_eq!(map("Shaders/Post.wgsl"), Some("wgsl"));
        assert_eq!(map("Assets/Hero.shader"), None, "profile code has no shader assets");
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

    #[test]
    fn node_module_extensions_are_indexed_and_imports_bind_without_views() {
        let dir = temp_project("node-module-extensions");
        write(&dir.join(".codegraph233.json"), r#"{"version":1,"profile":"code","roots":[{"key":"app","path":".","role":"frontend"}],"languages":["typescript"]}"#);
        write(&dir.join("codec.cjs"), "function encode(value) { return value; } module.exports = { encode };");
        write(&dir.join("entry.mjs"), "import { encode } from './codec.cjs'; export function render() { return encode(43); }");
        write(&dir.join("policy.cts"), "export function guard(): boolean { return true; }");
        write(&dir.join("helper.mts"), "import { guard } from './policy.cts'; export function check() { return guard(); }");
        let cfg = config::load(&dir).unwrap();
        let result = run(&cfg, &Options { jobs: 1, quiet: true, ..Default::default() }).unwrap();
        assert_eq!(result.files, 4);
        let edges = edge_set(&cfg.db_path());
        assert!(edges.contains(&("entry.render".into(), "codec.encode".into(), "calls".into())), "{edges:?}");
        assert!(edges.contains(&("helper.check".into(), "policy.guard".into(), "calls".into())), "{edges:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mixed_source_graph_preserves_imports_errors_and_incremental_health() {
        let dir = temp_project("mixed-source");
        fs::remove_file(dir.join("A.cs")).unwrap();
        fs::remove_file(dir.join("B.cs")).unwrap();
        write(&dir.join(".codegraph233.json"), r#"{"version":1,"profile":"code","roots":[{"key":"app","path":".","role":"frontend"}],"languages":["auto"]}"#);
        write(&dir.join("pkg/codec.py"), "def encode(value):\n    return value\n");
        write(&dir.join("pkg/entry.py"), "from .codec import encode as pack\ndef run():\n    return pack(43)\n");
        write(&dir.join("demo/A.java"), "package demo; class A { static int work(int n) { return n; } }");
        write(&dir.join("demo/B.java"), "package demo; class B { int run() { return A.work(43); } }");
        write(&dir.join("bad.py"), "def broken(:\n");
        let cfg=config::load(&dir).unwrap();
        let first=run(&cfg,&opts()).unwrap();
        assert!(first.parse_errors > 0);
        let edges=edge_set(&cfg.db_path());
        assert!(edges.contains(&("pkg.entry.run".into(),"pkg.codec.encode".into(),"calls".into())),"{edges:?}");
        assert!(edges.contains(&("demo.B.run".into(),"demo.A.work".into(),"calls".into())),"{edges:?}");
        let idle=run(&cfg,&opts()).unwrap();
        assert_eq!(idle.changed,0);
        assert_eq!(idle.parse_errors,first.parse_errors);
        assert_eq!(idle.unresolved,first.unresolved);
        write(&dir.join("bad.py"), "def repaired():\n    return 43\n");
        let repaired=run(&cfg,&opts()).unwrap();
        assert_eq!(repaired.parse_errors,0);
        fs::remove_file(dir.join("pkg/codec.py")).unwrap();
        let removed=run(&cfg,&opts()).unwrap();
        assert_eq!(removed.removed,1);
        assert!(removed.unresolved > repaired.unresolved);
        let idle=run(&cfg,&opts()).unwrap();
        assert_eq!(idle.unresolved,removed.unresolved);
        let _=fs::remove_dir_all(dir);
    }

    #[test]
    fn explicit_c_annotations_fix_real_declarations_and_invalidate_cached_errors() {
        let dir = temp_project("c-annotations");
        fs::remove_file(dir.join("A.cs")).unwrap();
        fs::remove_file(dir.join("B.cs")).unwrap();
        let config = r#"{"version":1,"profile":"code","roots":[{"key":"app","path":"."}],"languages":["c"]}"#;
        write(&dir.join(".codegraph233.json"),config);
        write(&dir.join("codec.c"),"#define SE_API __declspec(dllexport)\n#define SE_CALL __cdecl\nstatic int work(void) { return 43; }\nSE_API int SE_CALL codec_begin(void) { return work(); }\n");
        let cfg=config::load(&dir).unwrap();
        assert!(run(&cfg,&opts()).unwrap().parse_errors > 0);
        write(&dir.join(".codegraph233.json"), &config.replace("\"languages\":[\"c\"]", "\"languages\":[\"c\"],\"c_annotations\":[\"SE_API\",\"SE_CALL\"]"));
        let cfg=config::load(&dir).unwrap();
        assert!(status(&cfg).stale);
        let parsed=run(&cfg,&opts()).unwrap();
        assert_eq!(parsed.parse_errors,0);
        assert!(edge_set(&cfg.db_path()).contains(&("codec_begin".into(),"work".into(),"calls".into())));
        let store=Store::open(&cfg.db_path(),false).unwrap();
        let signature:String=store.conn.query_row("SELECT signature FROM symbols WHERE name='codec_begin'",[],|r|r.get(0)).unwrap();
        assert!(signature.contains("SE_API") && signature.contains("SE_CALL"));
        drop(store);
        write(&dir.join(".codegraph233.json"),&config.replace("\"languages\":[\"c\"]","\"languages\":[\"c\"],\"c_annotations\":[\"not a macro\"]"));
        assert!(config::load(&dir).unwrap_err().contains("c_annotations"));
        let _=fs::remove_dir_all(dir);
    }
    #[test]
    fn indexes_unreal_and_godot_project_assets() {
        let dir = std::env::temp_dir().join(format!("cg233-engine-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        write(
            &dir.join(".codegraph233.json"),
            r#"{"version":1,"profile":"full","roots":[{"key":"app","path":".","role":"frontend"}],"languages":["auto"]}"#,
        );
        write(
            &dir.join("MyGame.uproject"),
            r#"{"FileVersion":3,"EngineAssociation":"5.4","Modules":[{"Name":"MyGame"}],"Plugins":[{"Name":"EnhancedInput"}]}"#,
        );
        write(
            &dir.join("Config/DefaultEngine.ini"),
            "[/Script/EngineSettings.GameMapsSettings]\nGameDefaultMap=/Game/Maps/Main.Main\n",
        );
        let mut uasset = 0xC1832A9Eu32.to_le_bytes().to_vec();
        uasset.extend_from_slice(&[0u8; 12]);
        write_bytes(&dir.join("Content/Hero.uasset"), &uasset);
        write(
            &dir.join("scenes/Main.tscn"),
            "[gd_scene load_steps=2 format=3]\n\n[ext_resource type=\"Script\" path=\"res://src/player.gd\" id=\"1_abc\"]\n\n[node name=\"Player\" type=\"CharacterBody2D\"]\nscript = ExtResource(\"1_abc\")\n",
        );
        write(
            &dir.join("project.godot"),
            "[application]\n\nconfig/name=\"Game\"\nrun/main_scene=\"res://scenes/Main.tscn\"\n\n[autoload]\n\nPlayer=\"*res://src/player.gd\"\n",
        );
        write(
            &dir.join("src/player.gd"),
            "class_name Player\nextends Node\n\nfunc _ready() -> void:\n\tpass\n",
        );

        let cfg = config::load(&dir).unwrap();
        let stats = run(&cfg, &opts()).unwrap();
        assert_eq!(stats.parse_errors, 0);
        let store = Store::open(&cfg.db_path(), false).unwrap();
        let rows: Vec<(String, String, String)> = store
            .conn
            .prepare("SELECT src, dst, kind FROM asset_edges")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .flatten()
            .collect();
        let has = |src: &str, dst: &str, kind: &str| {
            rows.iter().any(|r| r.0 == src && r.1 == dst && r.2 == kind)
        };
        assert!(
            has("asset:app:MyGame.uproject", "ue_module:MyGame", "contains_module"),
            "{rows:?}"
        );
        assert!(has(
            "asset:app:MyGame.uproject",
            "ue_plugin:EnhancedInput",
            "uses_plugin"
        ));
        assert!(has(
            "asset:app:Config/DefaultEngine.ini",
            "uegame:Maps/Main.Main",
            "ue_reference"
        ));
        assert!(has(
            "asset:app:scenes/Main.tscn",
            "asset:app:src/player.gd",
            "references"
        ));
        assert!(has(
            "asset:app:project.godot",
            "asset:app:scenes/Main.tscn",
            "main_scene"
        ));
        assert!(has(
            "asset:app:project.godot",
            "asset:app:src/player.gd",
            "autoload"
        ));
        let kinds: Vec<String> = store
            .conn
            .prepare("SELECT kind FROM assets")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();
        for kind in ["ue_project", "ue_config", "ue_asset", "godot_scene", "godot_project"] {
            assert!(kinds.iter().any(|k| k == kind), "missing {kind}: {kinds:?}");
        }
        drop(store);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extra_extensions_route_discovery_and_languages() {
        let dir = temp_project("extra-ext");
        write(
            &dir.join(".codegraph233.json"),
            r#"{"version":1,"profile":"code","roots":[{"key":"app","path":".","role":"frontend"}],"languages":["gdscript"],"extra_extensions":{"gd2":"gdscript"}}"#,
        );
        write(
            &dir.join("Hero.gd2"),
            "class_name Hero\nextends Node\n\nfunc ping() -> void:\n\tpass\n",
        );
        let cfg = config::load(&dir).unwrap();
        assert_eq!(lang_of("Hero.gd2", &cfg, &[]), Some("gdscript"));
        let stats = run(&cfg, &opts()).unwrap();
        assert_eq!(stats.parse_errors, 0);
        assert!(stats.files >= 1);
        let store = Store::open(&cfg.db_path(), false).unwrap();
        let n: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM symbols WHERE qualname='Hero.ping'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        drop(store);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn indexes_art_and_audio_pipeline_assets() {
        let dir = temp_project("pipeline");
        write(
            &dir.join(".codegraph233.json"),
            r#"{"version":1,"profile":"full","roots":[{"key":"app","path":".","role":"frontend"}],"languages":["auto"]}"#,
        );
        write(
            &dir.join("Audio/Main.mixer"),
            "%YAML 1.1\n--- !u!244 &1\nAudioMixerController:\n  m_Name: Main\n  m_Children:\n  - {fileID: 2}\n--- !u!245 &2\nAudioMixerGroupController:\n  m_Name: Music\n  m_ParentGroup: {fileID: 1}\n",
        );
        write(
            &dir.join("Wwise/Events.wwu"),
            "<WwiseDocument>\n  <Event Name=\"Play_Footstep\" />\n  <SoundBank Name=\"MainBank\" />\n  <SwitchGroup Name=\"Surface\" />\n</WwiseDocument>\n",
        );
        let mut bank = Vec::new();
        bank.extend_from_slice(b"BKHD");
        bank.extend_from_slice(&4u32.to_le_bytes());
        bank.extend_from_slice(&150u32.to_le_bytes());
        bank.extend_from_slice(&1234u32.to_le_bytes());
        bank.extend_from_slice(&0u32.to_le_bytes());
        write_bytes(&dir.join("Wwise/Main.bnk"), &bank);
        write(
            &dir.join("FMOD/Game.fspro"),
            r#"{"name":"Game","events":[{"name":"event:/Footstep"}],"buses":[{"name":"bus:/SFX"}],"banks":[{"name":"Master"}]}"#,
        );
        write_bytes(
            &dir.join("FMOD/Desktop.strings"),
            b"event:/Footstep\0event:/Impact\0",
        );
        let mut fsb = Vec::new();
        fsb.extend_from_slice(b"FSB5");
        fsb.extend_from_slice(&1u32.to_le_bytes());
        fsb.extend_from_slice(&2u32.to_le_bytes());
        fsb.extend_from_slice(&0u32.to_le_bytes());
        fsb.extend_from_slice(&0u32.to_le_bytes());
        fsb.extend_from_slice(&16u32.to_le_bytes());
        fsb.extend_from_slice(&0u32.to_le_bytes());
        write_bytes(&dir.join("FMOD/Desktop.bank"), &fsb);
        write(
            &dir.join("Art/Hero.gltf"),
            r#"{"asset":{"version":"2.0","generator":"test"},"nodes":[{"name":"Root"}],"meshes":[{"name":"Body","primitives":[{"material":0}]}],"materials":[{"name":"Skin"}],"images":[{"uri":"textures/hero.png"}],"textures":[{"source":0}],"animations":[{"name":"Idle"}],"skins":[{"name":"Rig"}]}"#,
        );
        let mut fbx = Vec::new();
        fbx.extend_from_slice(b"Kaydara FBX Binary  \x00");
        fbx.extend_from_slice(&[0x1A, 0x00]);
        fbx.extend_from_slice(&7400u32.to_le_bytes());
        write_bytes(&dir.join("Art/Hero.fbx"), &fbx);
        write(
            &dir.join("Packages/manifest.json"),
            r#"{"dependencies":{"com.unity.inputsystem":"1.7.0","com.unity.render-pipelines.universal":"14.0.0"}}"#,
        );

        let cfg = config::load(&dir).unwrap();
        let stats = run(&cfg, &opts()).unwrap();
        assert_eq!(stats.parse_errors, 0);
        let store = Store::open(&cfg.db_path(), false).unwrap();
        let kinds: Vec<String> = store
            .conn
            .prepare("SELECT DISTINCT kind FROM assets")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();
        for kind in [
            "audio_mixer_group",
            "wwise_event",
            "wwise_bank",
            "fmod_event",
            "fmod_bank",
            "gltf_scene",
            "gltf_mesh",
            "gltf_material",
            "fbx",
            "upm_manifest",
            "upm_package",
        ] {
            assert!(kinds.iter().any(|k| k == kind), "missing {kind}: {kinds:?}");
        }
        let edges: Vec<(String, String, String)> = store
            .conn
            .prepare("SELECT src, dst, kind FROM asset_edges")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .flatten()
            .collect();
        for kind in ["routes_to", "uses_material", "references"] {
            assert!(edges.iter().any(|e| e.2 == kind), "missing edge {kind}: {edges:?}");
        }
        drop(store);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn indexes_design_docs_narrative_and_code_markers() {
        let dir = temp_project("design");
        write(
            &dir.join(".codegraph233.json"),
            r#"{"version":1,"profile":"full","roots":[{"key":"app","path":".","role":"frontend"}],"languages":["auto"]}"#,
        );
        write(
            &dir.join("docs/Combat.md"),
            "---\ntitle: Combat\n---\n# Combat\n@id: REQ-COMBAT\n@code: src/Combat.cs#damage\n@audio: Audio/hit.wav\n@story: Story/quest.yarn#start\nSee [[REQ-UI]].\n",
        );
        write(
            &dir.join("src/Combat.cs"),
            "namespace Demo {\n  // @design:REQ-COMBAT\n  class Combat { void damage() {} }\n}\n",
        );
        write(
            &dir.join("Story/quest.yarn"),
            "title: start\n---\nHello #greeting\n<<jump ending>>\n===\ntitle: ending\n---\nBye\n",
        );
        write(&dir.join("Story/branch.ink"), "=== start ===\n-> ending\n=== ending ===\n");
        write(
            &dir.join("Media/sub.srt"),
            "1\n00:00:01,000 --> 00:00:02,000\nHi\n",
        );
        write_bytes(&dir.join("Media/trailer.webm"), b"\x1A\x45\xDF\xA3webm");

        let cfg = config::load(&dir).unwrap();
        let stats = run(&cfg, &opts()).unwrap();
        assert_eq!(stats.parse_errors, 0);
        let store = Store::open(&cfg.db_path(), false).unwrap();
        let kinds: Vec<String> = store
            .conn
            .prepare("SELECT DISTINCT kind FROM assets")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();
        for kind in [
            "design_doc",
            "design_section",
            "design_requirement",
            "yarn_script",
            "yarn_node",
            "ink_script",
            "ink_knot",
            "subtitle",
            "video",
        ] {
            assert!(kinds.iter().any(|k| k == kind), "missing {kind}: {kinds:?}");
        }
        let edges: Vec<(String, String, String)> = store
            .conn
            .prepare("SELECT src, dst, kind FROM asset_edges")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .flatten()
            .collect();
        for kind in [
            "design_code",
            "design_audio",
            "design_story",
            "design_link",
            "implements",
            "jumps_to",
        ] {
            assert!(edges.iter().any(|e| e.2 == kind), "missing edge {kind}: {edges:?}");
        }
        assert!(edges.iter().any(|e| e.0 == "asset:app:src/Combat.cs"
            && e.1 == "design:app:REQ-COMBAT"
            && e.2 == "implements"));
        assert!(edges.iter().any(|e| e.0 == "design:app:REQ-COMBAT"
            && e.1 == "asset:app:src/Combat.cs"
            && e.2 == "design_code"));
        drop(store);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn indexes_localization_text_assets_and_design_text_links() {
        let dir = temp_project("localization");
        write(
            &dir.join(".codegraph233.json"),
            r#"{"version":1,"profile":"full","roots":[{"key":"app","path":".","role":"frontend"}],"languages":["auto"]}"#,
        );
        write(
            &dir.join("docs/Text.md"),
            "# Text\n@id: REQ-TEXT\n@text: Loc/Strings.po#MENU_PLAY\n",
        );
        write(
            &dir.join("Loc/Strings.po"),
            "msgid \"\"\nmsgstr \"\"\n\"Language: fr\\n\"\n\nmsgctxt \"MENU_PLAY\"\nmsgid \"Play\"\nmsgstr \"Jouer\"\n\nmsgid \"MENU_QUIT\"\nmsgstr \"\"\n",
        );
        write(
            &dir.join("Loc/Strings.xliff"),
            r#"<?xml version="1.0"?><xliff version="1.2"><file source-language="en" target-language="de"><body><trans-unit id="MENU_START"><source>Start</source><target>Start</target></trans-unit><trans-unit id="MENU_QUIT" state="needs-translation"><source>Quit</source><target></target></trans-unit></body></file></xliff>"#,
        );
        write(
            &dir.join("Loc/Strings.de.resx"),
            r#"<root><data name="MENU_HELP" xml:space="preserve"><value>Hilfe</value></data></root>"#,
        );
        write(
            &dir.join("Loc/strings.arb"),
            r#"{"@@locale":"ja","MENU_HELP":"ヘルプ"}"#,
        );

        let cfg = config::load(&dir).unwrap();
        let stats = run(&cfg, &opts()).unwrap();
        assert_eq!(stats.parse_errors, 0);
        let store = Store::open(&cfg.db_path(), false).unwrap();
        let kinds: Vec<String> = store
            .conn
            .prepare("SELECT DISTINCT kind FROM assets")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();
        for kind in ["localization", "localization_entry", "design_requirement"] {
            assert!(kinds.iter().any(|k| k == kind), "missing {kind}: {kinds:?}");
        }
        let entry_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM assets WHERE kind='localization_entry'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(entry_count, 6, "PO 2 + XLIFF 2 + RESX 1 + ARB 1");
        let untranslated: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM assets WHERE kind='localization_entry'
                 AND meta LIKE '%\"status\":\"untranslated\"%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(untranslated, 2);
        let edges: Vec<(String, String, String, String)> = store
            .conn
            .prepare("SELECT src, dst, kind, COALESCE(detail,'') FROM asset_edges")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .flatten()
            .collect();
        assert!(edges.iter().any(|e| e.0 == "design:app:REQ-TEXT"
            && e.1 == "asset:app:Loc/Strings.po"
            && e.2 == "design_text"
            && e.3 == "MENU_PLAY"));
        drop(store);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn localization_report_and_audio_events_are_indexed() {
        let dir = temp_project("localization-qa");
        write(
            &dir.join(".codegraph233.json"),
            r#"{"version":1,"profile":"full","roots":[{"key":"app","path":".","role":"frontend"}],"languages":["auto"]}"#,
        );
        write(
            &dir.join("Loc/Strings.po"),
            "msgid \"\"\nmsgstr \"\"\n\"Language: de\\n\"\n\nmsgid \"Press {0} to start\"\nmsgstr \"Drücke zum Starten\"\n\nmsgid \"Go\"\nmsgstr \"Dies ist eine viel zu lange Übersetzung für einen kleinen Button\"\n",
        );
        write(
            &dir.join("Wwise/Events.wwu"),
            r#"<?xml version="1.0"?><WwiseDocument><Event Name="Play_Footstep"/></WwiseDocument>"#,
        );
        write(
            &dir.join("Localization/Strings.csv"),
            "Key,SourceString,en,de\nMENU_PLAY,Play,Play,Spielen\n",
        );
        write(
            &dir.join("src/Audio.cs"),
            "class Audio { void Play() { AkSoundEngine.PostEvent(\"Play_Footstep\", this); } }\n",
        );

        let cfg = config::load(&dir).unwrap();
        let stats = run(&cfg, &opts()).unwrap();
        assert_eq!(stats.parse_errors, 0);
        let eng = crate::query::Engine::open(&cfg).unwrap();

        let report = eng.localization_report(Some("app"), 50).unwrap();
        assert!(report["entry_count"].as_i64().unwrap() >= 4);
        assert!(report["issue_count"].as_i64().unwrap() >= 2);
        assert!(report["by_kind"]["placeholder_mismatch"].as_i64().unwrap() >= 1);
        assert!(report["by_kind"]["overlong"].as_i64().unwrap() >= 1);
        let issues = report["issues"].as_array().unwrap();
        assert!(issues.iter().any(|i| i["kind"] == "placeholder_mismatch"
            && i["key"] == "Press {0} to start"));
        assert!(report["languages"]["de"].as_i64().unwrap() >= 3);

        let events = eng.audio_events(Some("app"), 50).unwrap();
        assert_eq!(events["event_count"].as_i64().unwrap(), 1);
        assert_eq!(events["usage_count"].as_i64().unwrap(), 1);
        assert_eq!(events["resolved"].as_i64().unwrap(), 1);
        let first = &events["events"][0];
        assert_eq!(first["event"], "Play_Footstep");
        assert_eq!(first["asset"]["kind"], "wwise_event");
        assert_eq!(first["sites"][0]["path"], "src/Audio.cs");

        drop(eng);
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
