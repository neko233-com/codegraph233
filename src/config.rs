use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CONFIG_NAME: &str = ".codegraph233.json";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Root {
    pub key: String,
    pub path: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FeatureSet {
    pub code: Option<bool>,
    pub type_deps: Option<bool>,
    pub prefabs: Option<bool>,
    pub shaders: Option<bool>,
    pub assets: Option<bool>,
    pub meta: Option<bool>,
    pub config_tables: Option<bool>,
    pub guid_map: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RawConfig {
    #[serde(default = "one")]
    pub version: i32,
    #[serde(default = "auto")]
    pub strategy: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub roots: Vec<Root>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub config_dirs: Vec<String>,
    #[serde(default)]
    pub features: FeatureSet,
    #[serde(default)]
    pub guid_map: Vec<String>,
    #[serde(default = "jvm_release")]
    pub jvm_release: u32,
    /// Explicit declaration annotations from this project's C/C++ headers.
    /// Names only: no replacement text or preprocessor execution.
    #[serde(default)]
    pub c_annotations: Vec<String>,
}

pub fn jvm_release() -> u32 { 8 }

fn one() -> i32 {
    1
}
fn auto() -> String {
    "auto".to_string()
}

#[derive(Debug, Clone)]
pub struct Features {
    pub code: bool,
    pub type_deps: bool,
    pub prefabs: bool,
    pub shaders: bool,
    pub assets: bool,
    pub meta: bool,
    pub config_tables: bool,
    pub guid_map: bool,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub dir: PathBuf,
    pub file: PathBuf,
    pub raw: RawConfig,
    pub features: Features,
}

/// Code languages with a dedicated source frontend. `jvm` covers CLASS/JAR
/// bytecode; the rest are source ASTs.
pub const CODE_LANGUAGES: &[&str] = &[
    "csharp", "go", "rust", "typescript", "lua", "jvm", "java", "c", "cpp",
    "python", "powershell", "gdscript", "kotlin", "swift", "objc", "dart",
    "haxe", "gml", "hlsl", "glsl", "metal", "wgsl",
];

/// True for language ids that are parsed as source code (not assets/configs).
pub fn is_code_lang(lang: &str) -> bool {
    CODE_LANGUAGES.contains(&lang)
}

pub const DEFAULT_EXCLUDE: &[&str] = &[
    ".git", ".svn", ".hg", "Library", "Temp", "Logs", "obj", "Build", "Builds",
    "node_modules", ".codely-cli", ".codex", ".cursor", ".codegraph233",
    "Bundles", "StreamingAssets", "__pycache__", ".vs", ".idea", "DerivedDataCache",
    "Binaries", "Intermediate", "Saved", ".gradle", "UserSettings", "MemoryCaptures",
    "Recordings", ".utmp", "crashlytics-build", ".codely.packages", ".codely",
    "backup", "_backup", "备份", "Documentation~", "Samples~", "Tests~",
];

/// Walk up from `start` to the first directory containing `.codegraph233.json`.
pub fn find_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    if cur.is_file() {
        cur.pop();
    }
    loop {
        if cur.join(CONFIG_NAME).is_file() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

pub fn load(start: &Path) -> Result<Config, String> {
    let dir = find_root(start).ok_or_else(|| {
        format!(
            "no {} found from {} (run `codegraph233 init`)",
            CONFIG_NAME,
            start.display()
        )
    })?;
    let file = dir.join(CONFIG_NAME);
    let text = std::fs::read_to_string(&file).map_err(|e| format!("read config: {e}"))?;
    let mut raw: RawConfig =
        serde_json::from_str(&text).map_err(|e| format!("parse config: {e}"))?;
    if raw.c_annotations.len() > 32 || raw.c_annotations.iter().any(|name| {
        name.is_empty() || name.len() > 64 || !name.bytes().enumerate().all(|(i,b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
    }) {
        return Err("c_annotations requires at most 32 C identifier names of 1-64 ASCII bytes".into());
    }
    if raw.roots.is_empty() {
        raw.roots.push(Root {
            key: "client".into(),
            path: ".".into(),
            role: "frontend".into(),
        });
    }
    let features = resolve_features(&raw);
    Ok(Config {
        dir,
        file,
        raw,
        features,
    })
}

pub fn resolve_features(raw: &RawConfig) -> Features {
    let profile = raw.profile.to_ascii_lowercase();
    let (mut f, meta_default) = match profile.as_str() {
        "code" => (
            Features {
                code: true,
                type_deps: true,
                prefabs: false,
                shaders: false,
                assets: false,
                meta: false,
                config_tables: false,
                guid_map: false,
            },
            false,
        ),
        "minimal" => (
            Features {
                code: true,
                type_deps: true,
                prefabs: true,
                shaders: false,
                assets: false,
                meta: false,
                config_tables: false,
                guid_map: false,
            },
            false,
        ),
        "game" => (
            Features {
                code: true,
                type_deps: true,
                prefabs: true,
                shaders: true,
                assets: true,
                meta: false,
                config_tables: true,
                guid_map: true,
            },
            false,
        ),
        _ => (
            Features {
                code: true,
                type_deps: true,
                prefabs: true,
                shaders: true,
                assets: true,
                meta: true,
                config_tables: true,
                guid_map: true,
            },
            true,
        ),
    };
    let _ = meta_default;
    let o = &raw.features;
    if let Some(v) = o.code {
        f.code = v;
    }
    if let Some(v) = o.type_deps {
        f.type_deps = v;
    }
    if let Some(v) = o.prefabs {
        f.prefabs = v;
    }
    if let Some(v) = o.shaders {
        f.shaders = v;
    }
    if let Some(v) = o.assets {
        f.assets = v;
    }
    if let Some(v) = o.meta {
        f.meta = v;
    }
    if let Some(v) = o.config_tables {
        f.config_tables = v;
    }
    if let Some(v) = o.guid_map {
        f.guid_map = v;
    }
    f
}

impl Config {
    pub fn abs_root(&self, r: &Root) -> PathBuf {
        let p = Path::new(&r.path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.dir.join(p)
        }
    }

    pub fn db_path(&self) -> PathBuf {
        self.dir.join(".codegraph233").join("index.db")
    }

    /// Effective languages: empty -> csharp; "auto" -> all implemented.
    pub fn languages(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .raw
            .languages
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if out.is_empty() {
            return vec!["csharp".into()];
        }
        if out.iter().any(|s| s == "auto") {
            return CODE_LANGUAGES.iter().map(|s| s.to_string()).collect();
        }
        out.dedup();
        out
    }

    pub fn language_enabled(&self, lang: &str) -> bool {
        self.languages().iter().any(|l| l == lang || lang == "jvm" && l == "java")
    }

    pub fn excludes(&self) -> Vec<String> {
        let mut out: Vec<String> = DEFAULT_EXCLUDE.iter().map(|s| s.to_string()).collect();
        out.extend(self.raw.exclude.iter().cloned());
        out
    }
}

/// Exclude check: bare names match directory basenames, values with a slash
/// match root-relative prefixes.
pub fn is_excluded(rel: &str, base: &str, excludes: &[String]) -> bool {
    for ex in excludes {
        if ex.is_empty() {
            continue;
        }
        if ex.contains('/') || ex.contains('\\') {
            let ex = ex.replace('\\', "/");
            let ex = ex.trim_end_matches('/');
            if rel == ex || rel.starts_with(&format!("{ex}/")) {
                return true;
            }
        } else if base == ex {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cg233-config-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn auto_languages_cover_every_code_frontend() {
        let dir = temp_dir("auto");
        std::fs::write(
            dir.join(CONFIG_NAME),
            r#"{"version":1,"profile":"code","languages":["auto"]}"#,
        )
        .unwrap();
        let cfg = load(&dir).unwrap();
        let langs = cfg.languages();
        for lang in CODE_LANGUAGES {
            assert!(langs.iter().any(|l| l == lang), "auto must enable {lang}");
        }
        assert!(cfg.language_enabled("jvm"));
        assert!(cfg.language_enabled("gdscript"));
        assert!(cfg.language_enabled("wgsl"));
        assert!(is_code_lang("kotlin"));
        assert!(!is_code_lang("prefab"));
    }

    #[test]
    fn empty_languages_default_to_csharp_only() {
        let dir = temp_dir("empty");
        std::fs::write(dir.join(CONFIG_NAME), r#"{"version":1,"profile":"code"}"#).unwrap();
        let cfg = load(&dir).unwrap();
        assert_eq!(cfg.languages(), vec!["csharp".to_string()]);
        assert!(!cfg.language_enabled("jvm"));
        assert!(!cfg.language_enabled("rust"));
    }
}
