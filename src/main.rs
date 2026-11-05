mod assets;
mod binder;
mod config;
mod csharp;
mod go_lang;
mod indexer;
mod lexer;
mod mcp;
mod model;
mod projects;
mod query;
mod rust_lang;
mod setup;
mod store;
mod ts_lang;

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
         codegraph233 version\n"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");
    let rest: Vec<String> = args.iter().skip(2).cloned().collect();

    let root = arg_value(&rest, "--root")
        .map(PathBuf::from)
        .or_else(|| std::env::var("CODEGRAPH233_ROOT").ok().map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

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
                            "{{\"files\":{},\"changed\":{},\"removed\":{},\"symbols\":{},\"refs\":{},\"edges\":{},\"unresolved\":{},\"elapsed_ms\":{}}}",
                            stats.files, stats.changed, stats.removed, stats.symbols, stats.refs, stats.edges, stats.unresolved, stats.elapsed_ms
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
            let cfg = match config::load(&root) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };
            if let Err(e) = mcp::serve(cfg) {
                eprintln!("server error: {e}");
                std::process::exit(1);
            }
        }
        "version" | "--version" | "-v" => println!("codegraph233 0.2.0 (rust)"),
        _ => usage(),
    }
}
