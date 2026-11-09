//! Language registry: the single place that describes every frontend the
//! indexer knows about.
//!
//! Adding a language means adding one `LangSpec` here (for discovery, the
//! `languages` command and docs) and one parser arm in `indexer::parse_file`.
//! Projects can already map extra file extensions to an existing frontend with
//! `extra_extensions` in `.codegraph233.json`.

/// How the language is parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Hand-written tokenizer/parser.
    HandWritten,
    /// tree-sitter grammar.
    TreeSitter,
    /// Hand-written lexical frontend over the shared tokenizer.
    Lexical,
    /// JVM CLASS/JAR bytecode reader.
    Bytecode,
    /// Engine/asset format (not source code).
    Asset,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::HandWritten => "hand-written",
            Backend::TreeSitter => "tree-sitter",
            Backend::Lexical => "lexical",
            Backend::Bytecode => "bytecode",
            Backend::Asset => "asset",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LangSpec {
    pub id: &'static str,
    pub backend: Backend,
    pub extensions: &'static [&'static str],
    /// Statically typed languages get receiver/container/arity strictness in
    /// the binder. Dynamic frontends keep the looser name-based fallback.
    pub strict: bool,
}

pub const SPECS: &[LangSpec] = &[
    LangSpec { id: "csharp", backend: Backend::HandWritten, extensions: &["cs"], strict: false },
    LangSpec { id: "go", backend: Backend::HandWritten, extensions: &["go"], strict: false },
    LangSpec { id: "rust", backend: Backend::HandWritten, extensions: &["rs"], strict: false },
    LangSpec {
        id: "typescript",
        backend: Backend::HandWritten,
        extensions: &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"],
        strict: false,
    },
    LangSpec { id: "lua", backend: Backend::HandWritten, extensions: &["lua", "luau"], strict: false },
    LangSpec { id: "jvm", backend: Backend::Bytecode, extensions: &["class", "jar"], strict: false },
    LangSpec { id: "java", backend: Backend::TreeSitter, extensions: &["java"], strict: true },
    LangSpec { id: "c", backend: Backend::TreeSitter, extensions: &["c"], strict: true },
    LangSpec {
        id: "cpp",
        backend: Backend::TreeSitter,
        extensions: &["cc", "cpp", "cxx", "c++", "hpp", "hh", "hxx"],
        strict: true,
    },
    LangSpec { id: "python", backend: Backend::TreeSitter, extensions: &["py", "pyi"], strict: true },
    LangSpec {
        id: "powershell",
        backend: Backend::TreeSitter,
        extensions: &["ps1", "psm1", "psd1"],
        strict: false,
    },
    LangSpec { id: "gdscript", backend: Backend::TreeSitter, extensions: &["gd"], strict: false },
    LangSpec { id: "kotlin", backend: Backend::TreeSitter, extensions: &["kt", "kts"], strict: true },
    LangSpec { id: "swift", backend: Backend::TreeSitter, extensions: &["swift"], strict: true },
    LangSpec { id: "objc", backend: Backend::TreeSitter, extensions: &["m", "mm"], strict: true },
    LangSpec { id: "dart", backend: Backend::TreeSitter, extensions: &["dart"], strict: true },
    LangSpec { id: "haxe", backend: Backend::Lexical, extensions: &["hx"], strict: true },
    LangSpec { id: "gml", backend: Backend::Lexical, extensions: &["gml"], strict: false },
    LangSpec {
        id: "hlsl",
        backend: Backend::Lexical,
        extensions: &["hlsl", "cginc", "compute", "usf", "ush", "cg"],
        strict: false,
    },
    LangSpec {
        id: "glsl",
        backend: Backend::Lexical,
        extensions: &[
            "glsl", "vert", "frag", "geom", "comp", "tesc", "tese", "gdshader",
        ],
        strict: false,
    },
    LangSpec { id: "metal", backend: Backend::Lexical, extensions: &["metal"], strict: false },
    LangSpec { id: "wgsl", backend: Backend::Lexical, extensions: &["wgsl"], strict: false },
    // Engine/asset frontends are selectable as `extra_extensions` targets too.
    LangSpec { id: "prefab", backend: Backend::Asset, extensions: &["prefab", "unity"], strict: false },
    LangSpec { id: "shader", backend: Backend::Asset, extensions: &["shader"], strict: false },
    LangSpec { id: "config", backend: Backend::Asset, extensions: &["xlsx", "csv", "tsv"], strict: false },
    LangSpec { id: "meta", backend: Backend::Asset, extensions: &["meta"], strict: false },
    LangSpec { id: "asset", backend: Backend::Asset, extensions: &[], strict: false },
    LangSpec {
        id: "ue_project",
        backend: Backend::Asset,
        extensions: &["uproject"],
        strict: false,
    },
    LangSpec {
        id: "ue_plugin",
        backend: Backend::Asset,
        extensions: &["uplugin"],
        strict: false,
    },
    LangSpec {
        id: "ue_config",
        backend: Backend::Asset,
        extensions: &["ini"],
        strict: false,
    },
    LangSpec {
        id: "ue_asset",
        backend: Backend::Asset,
        extensions: &["uasset", "umap"],
        strict: false,
    },
    LangSpec {
        id: "godot_scene",
        backend: Backend::Asset,
        extensions: &["tscn", "tres"],
        strict: false,
    },
    LangSpec {
        id: "godot_project",
        backend: Backend::Asset,
        extensions: &["godot"],
        strict: false,
    },
    LangSpec {
        id: "unity_mixer",
        backend: Backend::Asset,
        extensions: &["mixer"],
        strict: false,
    },
    LangSpec {
        id: "wwise_workunit",
        backend: Backend::Asset,
        extensions: &["wwu"],
        strict: false,
    },
    LangSpec {
        id: "wwise_project",
        backend: Backend::Asset,
        extensions: &["wproj"],
        strict: false,
    },
    LangSpec {
        id: "wwise_bank",
        backend: Backend::Asset,
        extensions: &["bnk"],
        strict: false,
    },
    LangSpec {
        id: "wwise_media",
        backend: Backend::Asset,
        extensions: &["wem"],
        strict: false,
    },
    LangSpec {
        id: "fmod_project",
        backend: Backend::Asset,
        extensions: &["fspro"],
        strict: false,
    },
    LangSpec {
        id: "fmod_strings",
        backend: Backend::Asset,
        extensions: &["strings"],
        strict: false,
    },
    LangSpec {
        id: "fmod_bank",
        backend: Backend::Asset,
        extensions: &["bank"],
        strict: false,
    },
    LangSpec {
        id: "gltf",
        backend: Backend::Asset,
        extensions: &["gltf", "glb"],
        strict: false,
    },
    LangSpec {
        id: "aseprite",
        backend: Backend::Asset,
        extensions: &["ase", "aseprite"],
        strict: false,
    },
    LangSpec {
        id: "fbx",
        backend: Backend::Asset,
        extensions: &["fbx"],
        strict: false,
    },
    LangSpec {
        id: "design_doc",
        backend: Backend::Asset,
        extensions: &["md", "markdown"],
        strict: false,
    },
    LangSpec {
        id: "yarn_script",
        backend: Backend::Asset,
        extensions: &["yarn"],
        strict: false,
    },
    LangSpec {
        id: "ink_script",
        backend: Backend::Asset,
        extensions: &["ink"],
        strict: false,
    },
    LangSpec {
        id: "subtitle",
        backend: Backend::Asset,
        extensions: &["srt", "vtt"],
        strict: false,
    },
    LangSpec {
        id: "video",
        backend: Backend::Asset,
        extensions: &["webm", "mkv"],
        strict: false,
    },
    LangSpec {
        id: "localization_po",
        backend: Backend::Asset,
        extensions: &["po", "pot"],
        strict: false,
    },
    LangSpec {
        id: "localization_xliff",
        backend: Backend::Asset,
        extensions: &["xliff", "xlf"],
        strict: false,
    },
    LangSpec {
        id: "localization_resx",
        backend: Backend::Asset,
        extensions: &["resx"],
        strict: false,
    },
    LangSpec {
        id: "localization_arb",
        backend: Backend::Asset,
        extensions: &["arb"],
        strict: false,
    },
    // Path-based frontends expose no extension of their own.
    LangSpec {
        id: "upm_manifest",
        backend: Backend::Asset,
        extensions: &[],
        strict: false,
    },
    LangSpec {
        id: "localization_csv",
        backend: Backend::Asset,
        extensions: &[],
        strict: false,
    },
];

pub fn spec(id: &str) -> Option<&'static LangSpec> {
    SPECS.iter().find(|s| s.id == id)
}

/// Resolve a configured language id to the canonical static id.
pub fn canonical(id: &str) -> Option<&'static str> {
    spec(id.trim().to_ascii_lowercase().as_str()).map(|s| s.id)
}

pub fn is_strict(lang: &str) -> bool {
    spec(lang).map(|s| s.strict).unwrap_or(false)
}

pub fn extensions(id: &str) -> &'static [&'static str] {
    spec(id).map(|s| s.extensions).unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_language_has_a_registry_entry() {
        for lang in crate::config::CODE_LANGUAGES {
            let spec = spec(lang).unwrap_or_else(|| panic!("missing spec for {lang}"));
            assert_eq!(canonical(lang), Some(spec.id));
            assert!(!spec.extensions.is_empty(), "{lang} needs extensions");
        }
    }

    #[test]
    fn strict_languages_match_the_binder_contract() {
        let strict: Vec<&str> = SPECS.iter().filter(|s| s.strict).map(|s| s.id).collect();
        assert_eq!(
            strict,
            vec!["java", "c", "cpp", "python", "kotlin", "swift", "objc", "dart", "haxe"]
        );
        assert!(!is_strict("gdscript"));
        assert!(!is_strict("gml"));
        assert!(!is_strict("typescript"));
    }

    #[test]
    fn extensions_are_unique_within_and_across_frontends() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for s in SPECS {
            for ext in s.extensions {
                assert!(seen.insert(*ext), "extension {ext} is listed twice");
            }
        }
    }

    #[test]
    fn engine_asset_frontends_are_registered() {
        for id in [
            "ue_project",
            "ue_plugin",
            "ue_config",
            "ue_asset",
            "godot_scene",
            "godot_project",
        ] {
            assert_eq!(canonical(id), Some(id));
        }
    }
}
