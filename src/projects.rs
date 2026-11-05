//! User-level project registry so one MCP process can serve several repos.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub root: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub projects: Vec<Project>,
}

pub fn path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".codegraph233").join("projects.json")
}

pub fn load() -> Registry {
    let p = path();
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(_) => return Registry::default(),
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save(reg: &Registry) -> Result<(), String> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(reg).map_err(|e| e.to_string())?;
    std::fs::write(&p, format!("{text}\n")).map_err(|e| e.to_string())
}

fn normalize(root: &str) -> String {
    let p = Path::new(root);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    };
    let s = std::fs::canonicalize(&abs)
        .unwrap_or(abs)
        .to_string_lossy()
        .replace('\\', "/");
    // strip Windows verbatim prefix (\\?\)
    s.trim_start_matches("//?/").to_string()
}

pub fn add(reg: &mut Registry, root: &str) -> bool {
    let norm = normalize(root);
    if reg.projects.iter().any(|p| normalize(&p.root) == norm) {
        return false;
    }
    let name = Path::new(&norm)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| norm.clone());
    reg.projects.push(Project { root: norm, name });
    true
}

pub fn remove(reg: &mut Registry, root: &str) -> bool {
    let norm = normalize(root);
    let before = reg.projects.len();
    reg.projects.retain(|p| normalize(&p.root) != norm);
    before != reg.projects.len()
}

/// Resolve a selector (empty | name | root | path fragment) to a project root.
pub fn resolve(default_dir: &str, sel: &str) -> String {
    let sel = sel.trim();
    if sel.is_empty() {
        return default_dir.to_string();
    }
    let reg = load();
    let lower = sel.to_ascii_lowercase();
    for p in &reg.projects {
        if p.name.eq_ignore_ascii_case(sel) || normalize(&p.root) == normalize(sel) {
            return p.root.clone();
        }
    }
    for p in &reg.projects {
        if p.root.to_ascii_lowercase().contains(&lower) {
            return p.root.clone();
        }
    }
    if normalize(default_dir) == normalize(sel) {
        return default_dir.to_string();
    }
    sel.to_string()
}
