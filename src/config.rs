use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    /// Extension -> registered language id. Lets a project index extra
    /// suffixes (e.g. `"mm2": "objc"`) without a new parser. Insertion order is
    /// stable because this participates in the config identity hash.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra_extensions: BTreeMap<String, String>,
    /// Path-based relationship rules. They let a project link arbitrary inputs
    /// (audio stems, design folders, naming conventions) without a parser.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub link_rules: Vec<LinkRule>,
}

/// One declarative relationship: every indexed path matching `include` gets an
/// edge to `dst`. `dst` may contain `{stem}`, `{name}`, `{ext}`, `{dir}`,
/// `{path}` or `{root}`; a value without a scheme is treated as a project path.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LinkRule {
    pub include: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub dst: String,
    #[serde(default)]
    pub detail: Option<String>,
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
    /// Validated, canonical extension overrides.
    pub extra_extensions: BTreeMap<String, String>,
    /// Validated, bounded path-based relationship rules.
    pub link_rules: Vec<LinkRule>,
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
    let extra_extensions = normalize_extra_extensions(&raw.extra_extensions)?;
    let link_rules = validate_link_rules(&raw.link_rules)?;
    let features = resolve_features(&raw);
    Ok(Config {
        dir,
        file,
        raw,
        features,
        extra_extensions,
        link_rules,
    })
}

/// Validate project extension overrides. Keys are bare suffixes (a leading dot
/// is ignored) and values must name a registered frontend.
fn normalize_extra_extensions(
    input: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    if input.len() > 64 {
        return Err("extra_extensions supports at most 64 entries".into());
    }
    let mut out = BTreeMap::new();
    for (ext, target) in input {
        let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
        if ext.is_empty()
            || ext.len() > 16
            || !ext
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'))
        {
            return Err(format!(
                "extra_extensions key {ext:?} must be 1-16 ASCII letters, digits, '_', '+' or '-'"
            ));
        }
        let Some(id) = crate::langs::canonical(target) else {
            return Err(format!(
                "extra_extensions target {target:?} is not a registered language (run `codegraph233 languages`)"
            ));
        };
        out.insert(ext, id.to_string());
    }
    Ok(out)
}

/// Validate bounded, declarative path-based links. Templates are placeholders
/// only, so a typo fails at config load instead of silently creating dead edges.
fn validate_link_rules(rules: &[LinkRule]) -> Result<Vec<LinkRule>, String> {
    if rules.len() > 32 {
        return Err("link_rules supports at most 32 rules".into());
    }
    for (i, r) in rules.iter().enumerate() {
        if r.include.is_empty() || r.include.len() > 200 {
            return Err(format!("link_rules[{i}].include must be 1-200 bytes"));
        }
        if r.kind.is_empty()
            || r.kind.len() > 64
            || !r
                .kind
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
        {
            return Err(format!(
                "link_rules[{i}].kind must be 1-64 ASCII letters, digits, '_', '-', '.' or ':'"
            ));
        }
        if r.dst.is_empty() || r.dst.len() > 256 {
            return Err(format!("link_rules[{i}].dst must be 1-256 bytes"));
        }
        if r.detail.as_ref().map(|d| d.len() > 80).unwrap_or(false) {
            return Err(format!("link_rules[{i}].detail must be at most 80 bytes"));
        }
        let mut rest = r.dst.as_str();
        while let Some(open) = rest.find('{') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('}') else {
                return Err(format!("link_rules[{i}].dst has an unclosed placeholder"));
            };
            let key = &after[..close];
            if !matches!(key, "stem" | "name" | "ext" | "dir" | "path" | "root") {
                return Err(format!(
                    "link_rules[{i}].dst placeholder {{{key}}} is not one of stem/name/ext/dir/path/root"
                ));
            }
            rest = &after[close + 1..];
        }
        if rest.contains('}') {
            return Err(format!("link_rules[{i}].dst has a stray '}}'"));
        }
    }
    Ok(rules.to_vec())
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

    #[test]
    fn extra_extensions_are_normalized_and_validated() {
        let dir = temp_dir("extra-ext");
        std::fs::write(
            dir.join(CONFIG_NAME),
            r#"{"version":1,"profile":"code","extra_extensions":{".GD2":"gdscript","mm2":"objc","scene":"godot_scene"}}"#,
        )
        .unwrap();
        let cfg = load(&dir).unwrap();
        assert_eq!(cfg.extra_extensions.get("gd2").map(|s| s.as_str()), Some("gdscript"));
        assert_eq!(cfg.extra_extensions.get("mm2").map(|s| s.as_str()), Some("objc"));
        assert_eq!(cfg.extra_extensions.get("scene").map(|s| s.as_str()), Some("godot_scene"));

        let dir = temp_dir("extra-ext-bad");
        std::fs::write(
            dir.join(CONFIG_NAME),
            r#"{"version":1,"profile":"code","extra_extensions":{"xx":"not-a-language"}}"#,
        )
        .unwrap();
        let err = load(&dir).unwrap_err();
        assert!(err.contains("not a registered language"), "{err}");
    }
}
