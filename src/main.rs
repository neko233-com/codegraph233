mod assets;
mod binder;
mod config;
mod csharp;
mod engine_assets;
mod go_lang;
mod indexer;
mod jvm_facts;
mod jvm_binary;
mod langs;
mod source_ast;
mod quiet;
mod powershell_ast;
mod lexer;
mod lexer_langs;
mod lua_lang;
mod mcp;
mod model;
mod pipeline_assets;
mod projects;
mod query;
mod rust_lang;
mod setup;
mod store;
mod tree_langs;
mod ts_lang;
mod update;

use std::path::PathBuf;

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn has_flag(args: &[String], key: &str) -> bool {
    args.iter().any(|a| a == key)
}

fn usage() {
    eprintln!(
        "codegraph233 (Rust) — AST GraphRAG indexer\n\
         \n\
         codegraph233 index  [--root PATH] [--force] [--jobs N] [--profile P] [--safe] [--quiet]\n\
         codegraph233 status [--root PATH]\n\
         codegraph233 languages [--json]\n\
         codegraph233 compact [--root PATH]\n\
         codegraph233 register|unregister [--root PATH]\n\
         codegraph233 projects\n\
         codegraph233 install-mcp [--scope global|project|both]\n\
         codegraph233 update [--check] [--force] [--mirror URL] [--json]\n\
         codegraph233 update --enable|--disable [--interval HOURS] [--mirror URL]\n\
         codegraph233 update --status\n\
         codegraph233 version\n"
    );
}

fn main() {
    if let Err(error) = quiet::initialize() { eprintln!("error: {error}"); std::process::exit(1); }
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");
    let rest: Vec<String> = args.iter().skip(2).cloned().collect();

    let root = arg_value(&rest, "--root")
        .map(PathBuf::from)
        .or_else(|| std::env::var("CODEGRAPH233_ROOT").ok().map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Remove leftovers of earlier self-updates (renamed running binaries).
    update::cleanup_old_binaries();
    if matches!(cmd, "index" | "serve" | "mcp") && !has_flag(&rest, "--no-update") {
        std::thread::spawn(update::auto);
    }

    match cmd {
        "index" => {
            let mut cfg = match config::load(&root) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };
            if let Some(p) = arg_value(&rest, "--profile") {
                cfg.raw.profile = p;
                cfg.features = config::resolve_features(&cfg.raw);
            }
            let opts = indexer::Options {
                force: has_flag(&rest, "--force"),
                jobs: arg_value(&rest, "--jobs").and_then(|v| v.parse().ok()).unwrap_or(0),
                quiet: has_flag(&rest, "--quiet"),
                fast: !has_flag(&rest, "--safe"),
            };
            match indexer::run(&cfg, &opts) {
                Ok(stats) => {
                    if opts.quiet {
                        println!(
                            "{{\"files\":{},\"changed\":{},\"removed\":{},\"symbols\":{},\"refs\":{},\"edges\":{},\"unresolved\":{},\"parse_errors\":{},\"elapsed_ms\":{}}}",
                            stats.files, stats.changed, stats.removed, stats.symbols, stats.refs, stats.edges, stats.unresolved, stats.parse_errors, stats.elapsed_ms
                        );
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "status" => {
            let cfg = match config::load(&root) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };
            let st = indexer::status(&cfg);
            println!("config:   {}", cfg.file.display());
            println!("db:       {}", st.db);
            println!(
                "profile:  {}  languages: {:?}",
                if cfg.raw.profile.is_empty() { "full" } else { &cfg.raw.profile },
                cfg.languages()
            );
            if st.stale {
                println!("index:    stale schema (run `codegraph233 index` to rebuild)");
            }
            if !st.exists {
                println!("index:    not built (run `codegraph233 index`)");
                return;
            }
            println!(
                "counts:   files={} symbols={} refs={} edges={} assets={} guid={}",
                st.files, st.symbols, st.refs, st.edges, st.assets, st.guid_entries
            );
            if st.indexed_at > 0 {
                println!("indexed:  {}", st.indexed_at);
            }
        }
        "init" => {
            let backend = arg_value(&rest, "--backend");
            let profile = arg_value(&rest, "--profile").unwrap_or_else(|| "full".into());
            match setup::init(&root, backend.as_deref(), &profile) {
                Ok(p) => {
                    let (strategy, engine) = setup::detect_strategy(&root);
                    println!("wrote {}", p.display());
                    println!("detected: {strategy} {engine}");
                    println!("next: codegraph233 index");
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "install-mcp" => {
            let clients = arg_value(&rest, "--clients")
                .unwrap_or_else(|| "codex,cursor,vscode,air,cowork,workbuddy,project".into());
            let scope = arg_value(&rest, "--scope").unwrap_or_else(|| "global".into());
            let clean = has_flag(&rest, "--clean-project");
            match setup::install_mcp(&root, &clients, &scope, clean) {
                Ok(done) => {
                    for d in done {
                        println!("updated {d}");
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "parse" => {
            let target = match rest.first() {
                Some(p) => PathBuf::from(p),
                None => {
                    eprintln!("usage: codegraph233 parse <file> [--root PATH]");
                    std::process::exit(2);
                }
            };
            let cfg = match config::load(&root) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };
            let base = cfg.dir.clone();
            let rel = target
                .strip_prefix(&base)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| target.to_string_lossy().replace('\\', "/"));
            let parsed = indexer::parse_file_debug(&target, &rel, &cfg);
            let out = serde_json::to_string_pretty(&serde_json::json!({
                    "path": rel,
                    "symbols": parsed.symbols.iter().map(|s| serde_json::json!({
                        "kind": s.kind, "qualname": s.qualname, "container": s.container,
                        "start": s.start_line, "end": s.end_line,
                    })).collect::<Vec<_>>(),
                    "assets": parsed.assets.iter().map(|a| serde_json::json!({
                        "key": a.key, "kind": a.kind, "name": a.name, "meta": a.meta,
                    })).collect::<Vec<_>>(),
                    "asset_edges": parsed.asset_edges.iter().map(|e| serde_json::json!({
                        "src": e.src, "dst": e.dst, "kind": e.kind, "detail": e.detail,
                    })).collect::<Vec<_>>(),
                    "refs": parsed.refs.len(),
                    "errors": parsed.errors,
                }))
                .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"));
            println!("{out}");
        }
        "compact" => {
            let cfg = match config::load(&root) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };
            let db = cfg.db_path();
            if !db.is_file() {
                eprintln!("error: no index at {}", db.display());
                std::process::exit(1);
            }
            let before = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
            let store = match store::Store::open(&db, false) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };
            if let Err(e) = store.compact() {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
            drop(store);
            let after = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
            println!(
                "compacted: {:.1} MB -> {:.1} MB",
                before as f64 / 1_048_576.0,
                after as f64 / 1_048_576.0
            );
        }
        "register" => {
            let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let target = arg_value(&rest, "--root").map(PathBuf::from).unwrap_or(dir);
            let mut reg = projects::load();
            let added = projects::add(&mut reg, &target.to_string_lossy());
            if let Err(e) = projects::save(&reg) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
            println!("{} {}", if added { "registered" } else { "already registered" }, target.display());
        }
        "unregister" => {
            let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let target = arg_value(&rest, "--root").map(PathBuf::from).unwrap_or(dir);
            let mut reg = projects::load();
            let removed = projects::remove(&mut reg, &target.to_string_lossy());
            let _ = projects::save(&reg);
            println!("{} {}", if removed { "unregistered" } else { "not registered" }, target.display());
        }
        "languages" => {
            if has_flag(&rest, "--json") {
                let rows: Vec<_> = langs::SPECS
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "id": s.id,
                            "backend": s.backend.as_str(),
                            "extensions": s.extensions,
                            "strict": s.strict,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows).unwrap_or_default());
            } else {
                println!("{:<14} {:<12} {:<46} mode", "id", "backend", "extensions");
                for s in langs::SPECS {
                    let mode = if s.strict {
                        "strict"
                    } else if matches!(s.backend, langs::Backend::Asset) {
                        "asset"
                    } else {
                        "dynamic"
                    };
                    println!(
                        "{:<14} {:<12} {:<46} {}",
                        s.id,
                        s.backend.as_str(),
                        langs::extensions(s.id).join(","),
                        mode
                    );
                }
            }
        }
        "projects" => {
            let reg = projects::load();
            if reg.projects.is_empty() {
                println!("no registered projects; run `codegraph233 register` inside a project");
            }
            for p in &reg.projects {
                let st = config::load(std::path::Path::new(&p.root)).map(|c| indexer::status(&c)).ok();
                match st {
                    Some(s) => println!("{:<24} {:<52} files={} symbols={} indexed={}", p.name, p.root, s.files, s.symbols, s.exists),
                    None => println!("{:<24} {:<52} (config missing)", p.name, p.root),
                }
            }
        }
        "serve" | "mcp" => {
            let mut sel = arg_value(&rest, "--project").unwrap_or_default();
            if sel.is_empty() {
                sel = std::env::var("CODEGRAPH233_PROJECT").unwrap_or_default();
            }
            let mut cfg = match config::load(&root) {
                Ok(c) => c,
                Err(e) => {
                    // Global installs start `serve` from an arbitrary working
                    // directory; fall back to the registry when the cwd has no
                    // project config.
                    let reg = projects::load();
                    let pick = if !sel.trim().is_empty() {
                        reg.projects
                            .iter()
                            .find(|p| {
                                p.name.eq_ignore_ascii_case(sel.trim())
                                    || projects::same_root(&p.root, sel.trim())
                                    || p.root.to_ascii_lowercase().contains(&sel.trim().to_ascii_lowercase())
                            })
                            .cloned()
                    } else if reg.projects.len() == 1 {
                        reg.projects.first().cloned()
                    } else {
                        None
                    };
                    match pick {
                        Some(p) => match config::load(std::path::Path::new(&p.root)) {
                            Ok(c) => c,
                            Err(e2) => {
                                eprintln!("error: {e}; registry project {}: {e2}", p.root);
                                std::process::exit(1);
                            }
                        },
                        None => {
                            eprintln!(
                                "error: {e}\nset CODEGRAPH233_PROJECT, pass --root <project>, or run `codegraph233 install-mcp --scope project`"
                            );
                            std::process::exit(1);
                        }
                    }
                }
            };
            if let Some(p) = arg_value(&rest, "--profile") {
                cfg.raw.profile = p;
                cfg.features = config::resolve_features(&cfg.raw);
            }
            if let Err(e) = mcp::serve(cfg) {
                eprintln!("server error: {e}");
                std::process::exit(1);
            }
        }
        "update" => {
            let interval_flag = arg_value(&rest, "--interval");
            let interval = match interval_flag {
                Some(v) => match v.parse::<u64>() {
                    Ok(n) => Some(n),
                    Err(_) => {
                        eprintln!("error: --interval expects a number of hours");
                        std::process::exit(1);
                    }
                },
                None => None,
            };
            let enable = if has_flag(&rest, "--enable") {
                Some(true)
            } else if has_flag(&rest, "--disable") {
                Some(false)
            } else {
                None
            };
            if enable.is_some() || interval.is_some() {
                match update::apply_options(enable, interval, arg_value(&rest, "--mirror")) {
                    Ok(s) => println!("{}", update::settings_summary(&s)),
                    Err(e) => {
                        eprintln!("error: {e}");
                        std::process::exit(1);
                    }
                }
            } else if has_flag(&rest, "--status") {
                if let Err(e) = update::print_status() {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            } else {
                let check = has_flag(&rest, "--check");
                let force = has_flag(&rest, "--force");
                let json = has_flag(&rest, "--json");
                let mirror = arg_value(&rest, "--mirror");
                if let Err(e) = update::run(check, force, mirror, json) {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "version" | "--version" | "-v" => println!("codegraph233 {} (rust)", update::current_version()),
        _ => usage(),
    }
}
