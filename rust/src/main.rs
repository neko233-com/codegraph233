// codegraph233 front-end benchmark (Rust side).
// Usage: cg233-bench --root <project> [--jobs N]
mod csharp;

use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Instant;

const SKIP_DIRS: &[&str] = &[
    ".git", ".svn", ".hg", "Library", "Temp", "Logs", "obj", "Build", "Builds",
    "node_modules", ".codely-cli", ".codex", ".cursor", ".game-project-graph", ".codegraph233",
    "Bundles", "StreamingAssets", "__pycache__", ".vs", ".idea", "DerivedDataCache",
    "Binaries", "Intermediate", "Saved", ".gradle", "UserSettings", "MemoryCaptures",
    "Recordings", ".utmp", "crashlytics-build", ".codely.packages", ".codely", "backup",
    "_backup", "备份", "Documentation~", "Samples~", "Tests~",
];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if SKIP_DIRS.iter().any(|s| *s == name) {
                continue;
            }
            walk(&path, out);
        } else if name.to_ascii_lowercase().ends_with(".cs") {
            out.push(path);
        }
    }
}

fn bench_write(dir: &str, refs: usize, edges: usize) {
    use rusqlite::Connection;
    let path = std::path::Path::new(dir).join("bench-write-rs.db");
    let _ = std::fs::remove_file(&path);
    let total_start = std::time::Instant::now();
    let conn = Connection::open(&path).expect("open db");
    conn.execute_batch(
        "PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA temp_store=MEMORY;
         PRAGMA cache_size=-131072; PRAGMA mmap_size=268435456;
         CREATE TABLE refs (
           id INTEGER PRIMARY KEY, src_id INTEGER, src_key TEXT, file_id INTEGER, root TEXT,
           name TEXT, receiver TEXT, receiver_type TEXT, kind TEXT, namespace_hint TEXT,
           line INTEGER, arity INTEGER, target INTEGER, confidence TEXT);
         CREATE TABLE edges (src INTEGER, dst INTEGER, kind TEXT, line INTEGER, confidence TEXT);
         CREATE INDEX idx_edges_src ON edges(src);
         CREATE INDEX idx_edges_dst ON edges(dst);
         CREATE INDEX idx_refs_target ON refs(target);",
    )
    .expect("schema");

    let insert_start = std::time::Instant::now();
    {
        let tx = conn.unchecked_transaction().expect("tx");
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO refs
                     (src_id, src_key, file_id, root, name, receiver, receiver_type, kind, namespace_hint, line, arity, target, confidence)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
                )
                .expect("prep refs");
            for i in 0..refs {
                stmt.execute(rusqlite::params![
                    (i % 50000) as i64, "k", (i % 48000) as i64, "client", "Name", "", "", "calls",
                    "NS", (i % 900) as i64, 1i64, 0i64, ""
                ])
                .expect("insert ref");
            }
        }
        {
            let mut stmt = tx
                .prepare("INSERT INTO edges(src, dst, kind, line, confidence) VALUES (?1,?2,?3,?4,?5)")
                .expect("prep edges");
            for i in 0..edges {
                stmt.execute(rusqlite::params![
                    (i % 50000) as i64, (i % 40000) as i64, "calls", (i % 900) as i64, "exact"
                ])
                .expect("insert edge");
            }
        }
        tx.commit().expect("commit");
    }
    let insert_ms = insert_start.elapsed().as_millis();
    let total_ms = total_start.elapsed().as_millis();
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!(
        "{{\"refs\":{},\"edges\":{},\"insert_ms\":{},\"total_ms\":{},\"db_bytes\":{}}}",
        refs, edges, insert_ms, total_ms, bytes
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut jobs = 0usize;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                if i + 1 < args.len() {
                    root = PathBuf::from(&args[i + 1]);
                }
                i += 2;
            }
            "--jobs" => {
                if i + 1 < args.len() {
                    jobs = args[i + 1].parse().unwrap_or(0);
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    if args.iter().any(|a| a == "bench-write") {
        let refs = args.iter().position(|a| a == "--refs").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(1_000_000usize);
        let edges = args.iter().position(|a| a == "--edges").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(800_000usize);
        bench_write(root.to_str().unwrap_or("."), refs, edges);
        return;
    }
    if jobs == 0 {
        jobs = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    }

    let mut files: Vec<PathBuf> = Vec::new();
    walk(&root, &mut files);

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .expect("thread pool");

    let start = Instant::now();
    let (symbols, refs, bytes) = pool.install(|| {
        files
            .par_iter()
            .map(|p| match std::fs::read(p) {
                Ok(data) => {
                    let c = csharp::scan(&data);
                    (c.symbols, c.refs, data.len())
                }
                Err(_) => (0, 0, 0),
            })
            .reduce(|| (0, 0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2))
    });
    let ms = start.elapsed().as_millis();

    println!(
        "{{\"files\":{},\"symbols\":{},\"refs\":{},\"bytes\":{},\"elapsed_ms\":{},\"jobs\":{}}}",
        files.len(),
        symbols,
        refs,
        bytes,
        ms,
        jobs
    );
}
