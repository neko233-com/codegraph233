# Source graph support in 0.4.1-dev

This is an unreleased local development version, separate from stable 0.4.0.
`languages: ["auto"]` enables C#, Go, Rust, TypeScript/JavaScript, Lua (and
Luau), Java, Kotlin, GDScript, Swift, Objective-C, Dart, Haxe, GameMaker GML,
C, C++, Python, PowerShell, HLSL/GLSL/Metal/WGSL source graphs and JVM
bytecode, plus design/narrative/media assets (Markdown design docs, Yarn/Ink
scripts, subtitles and WebM/Matroska headers). Existing asset/configuration
frontends remain controlled by project profile/features.

| Input | Implementation and scope |
| --- | --- |
| `.java` | Tree-sitter types, constructors, methods, fields, explicit imports and calls |
| `.c`, `.cc`, `.cpp`, `.cxx`, `.c++`, `.h`, `.hpp`, `.hh`, `.hxx` | Namespaces, types, functions/declarators and calls; `.h` uses C++ when enabled, otherwise C |
| `.py`, `.pyi` | Lexical classes/functions, calls, explicit/relative import aliases and `self` |
| `.ps1`, `.psm1`, `.psd1` | Declarations/commands; PowerShell 7 official parser is a bounded fallback on grammar recovery |
| `.mjs`, `.cjs`, `.mts`, `.cts` | Existing TS/JS parser and relative module normalization on original files |
| `.gd` | Tree-sitter GDScript classes, inner classes, signals, enums, members, calls and `.new()` construction |
| `.kt`, `.kts` | Tree-sitter Kotlin packages/imports, classes/objects/interfaces/enums, companion members, properties, functions and calls |
| `.swift` | Tree-sitter Swift classes/structs/enums/actors/extensions, protocols, properties, init/functions, enum members and calls |
| `.m`, `.mm` | Tree-sitter Objective-C interfaces/categories/protocols, properties, selectors (`a:b:`), structs/enums, C functions and messages |
| `.dart` | Tree-sitter Dart imports/aliases, classes/enums/functions/methods/constructors, inheritance and calls |
| `.luau` | Lua frontend |
| `.hx` | Lexical Haxe packages/imports, classes/interfaces/enums/typedef/abstract, fields, methods/constructors and calls |
| `.gml` | Lexical GameMaker GML constructors, methods/static methods, plain functions and calls; a script with no top-level function is attributed to its file stem |
| `.hlsl`, `.cginc`, `.compute`, `.usf`, `.ush`, `.cg` | Lexical HLSL/Cg structs, functions and calls |
| `.glsl`, `.vert`, `.frag`, `.geom`, `.comp`, `.tesc`, `.tese`, `.gdshader` | Lexical GLSL/Godot shader structs, functions and calls |
| `.metal` | Lexical Metal structs/functions and calls |
| `.wgsl` | Lexical WGSL structs, `fn` entry points and calls |
| `.class`, `.jar`, `.cg-jvm.jsonl` | [Native bytecode reader and fact importer](jvm-facts.md) |
| `.uproject`, `.uplugin` | JSON project/plugin descriptors: engine association, modules and plugin dependencies as asset edges |
| `.ini` | Unreal config sections/keys, with `/Game/...` content paths and `/Script/...` class references |
| `.uasset`, `.umap` | Unreal package header only: magic, legacy/UE4/UE5 versions, package name table sample; no property serialization, decompression or blueprint logic |
| `.tscn`, `.tres` | Godot 4 text scenes/resources: nodes, external resources, sub-resources, `ExtResource` instances |
| `project.godot` | Godot project config: name, `run/main_scene`, `[autoload]` singletons |
| `.mixer` | Unity AudioMixer YAML: controller, groups, snapshots and effects, with `m_ParentGroup` routing edges |
| `.wwu`, `.wproj`, `.bnk`, `.wem` | Wwise workunits/projects/banks/media: events, switch/state groups, banks and bus objects; `.wem` RIFF/WAVE header only |
| `.fspro`, `.strings`, `.bank` | FMOD project/string tables/FSB5 banks: events, buses, VCAs, NUL-separated path lists, FSB5 header |
| `.gltf`, `.glb` | glTF scenes: nodes, meshes, materials, textures, images, animations and skins, with material/texture/URI edges |
| `.ase`, `.aseprite` | Aseprite layers and tags read from binary frame/chunk headers |
| `.fbx` | FBX header only: `Kaydara FBX Binary` magic + version, or ASCII `; FBX` banner |
| `.md`, `.markdown` | Design docs: front matter, headings, `@id` requirements, `@code`/`@asset`/`@audio`/`@video`/`@art`/`@story`/`@value` markers and `[[...]]` links; repo docs such as README/CHANGELOG are skipped |
| `.yarn` | Yarn nodes, `#tags`, options and `<<jump>>` target edges |
| `.ink` | Ink knots, stitches, tags, choices and `-> knot` target edges |
| `.srt`, `.vtt` | Subtitle cue count, duration and first text; no media decoding |
| `.webm`, `.mkv` | EBML header only: WebM/Matroska container probe; no tracks or payloads |
| `.po`, `.pot` | gettext PO/POT entries with context, source, target, fuzzy and untranslated status |
| `.xliff`, `.xlf` | XLIFF 1.2/2.0 units with id/source/target, `target-language` and state |
| `.resx` | .NET resource `data`/`value` pairs; locale inferred from `Strings.<locale>.resx` |
| `.arb` | Application Resource Bundle strings with `@@locale`; `@`-prefixed metadata ignored |
| `Packages/manifest.json`, `packages-lock.json` | Unity UPM package dependency graph |

These are static navigation aids, not complete compilers, type checkers, macro
preprocessors, runtime traces or source recovery tools. Known JNI/Windows
annotation identifier tokens (`JNICALL`, `JNIEXPORT`, `JNIIMPORT`, `WINAPI`,
`APIENTRY`, `CALLBACK`, `NTAPI`) are masked for C/C++ parsing with byte positions
preserved. Signatures retain original text; modifiers record normalization.
Custom macros, conditional compilation, templates and overload resolution can
leave missing/ambiguous relationships. Python dynamic imports, PowerShell module
execution, reflection, function-pointer targets and complete CommonJS inference
are not performed. Verify critical relationships against source.

For actual header-declared export/calling annotations, a project may explicitly
set `"c_annotations": ["SE_API", "SE_CALL"]`. At most 32 ASCII identifier names
of 1-64 bytes are accepted. Only matching identifier tokens are masked; source
strings/comments/directives remain intact, signatures retain the original
annotation prefix, and configuration changes invalidate prior diagnostics.
There is no replacement expression, macro expansion or compiler execution.
Without this configuration, unknown annotation errors remain visible.
A matched pair of split `#ifdef __cplusplus` / `extern "C" {` and closing
linkage guards is normalized for C++ headers while preserving source byte/line
positions. Comment-contained or unmatched patterns are retained. This is not
general conditional compilation.

New AST inputs have a 16 MiB source limit, two-second parser budget, two-million
node budget and depth bound 256. `cg233_project_health` reports reader/parser
versions, persistent `parse_errors` and the first 32 per-file diagnostics.
Diagnostics survive no-change indexes and disappear on repair/removal. Ordinary
JVM errors fail the artifact rather than accepting partial classes. Older
bespoke frontends retain their limitations; zero diagnostics is not proof of
complete semantic coverage.

The game-development frontends share the same bounds. GDScript, Kotlin, Swift,
Objective-C and Dart use bundled tree-sitter grammars; their declarations and
calls are navigation aids, not type checking. The Kotlin grammar is partial:
some one-line interface/object bodies surface recovery diagnostics, which are
reported rather than hidden. Haxe, GameMaker GML and HLSL/GLSL/Metal/WGSL use
the shared bounded lexer, so declarations are recognized from syntax at brace
depth and calls from call-shaped token runs with raw-source argument counts.
Shader source frontends are selected only when the `code` or `shaders` feature
is enabled; `.shader` assets keep the existing property/include/keyword parser.
Cross-language receiver resolution is deliberately strict for statically typed
languages (Java, C, C++, Python, Kotlin, Swift, Objective-C, Dart, Haxe);
GDScript and GML remain dynamic and fall back to name/container matching.

PowerShell fallback runs fixed trusted code via `pwsh -NoProfile -NonInteractive
-EncodedCommand`. Input is JSON on stdin passed only to the official
`Parser.ParseInput`; it is never evaluated or run. Child windows are hidden,
output bounded and a 10-second deadline kills the owned parser. Set
`CODEGRAPH233_PWSH` to an explicit PowerShell 7 executable if necessary. If absent,
retain the portable diagnostics and fallback failure. Other languages do not
require this runtime; cross-platform fallback execution is not tested here.

Engine project assets are read statically and never handed to an editor or
runtime. Unreal `.uasset`/`.umap` inputs are header-level views: they report the
package magic, saved engine versions and a bounded name-table sample when the
header is decodable, and otherwise keep a visible error; object properties,
exports, compressed chunks and blueprint bytecode are not parsed. Unreal
reflection macros (`UCLASS`, `UFUNCTION`, `UPROPERTY`, `GENERATED_BODY`, `*_API`
export macros and related tokens, including balanced parentheses) are masked for
C++ parsing with original byte/line positions preserved. Godot `.tscn`/`.tres`
parsing covers text resources only, and `.godot` project files are read as
`project.godot`; binary `.res`/`.scn` inputs are not decoded. Existing `.shader`
assets keep the ShaderLab property/include/keyword parser rather than the HLSL
source frontend.

Art and audio pipeline inputs are read statically. Unity `.mixer` files are
YAML documents whose group/snapshot/effect objects become assets and whose
`m_ParentGroup` links become `routes_to` edges (child to parent). Wwise `.wwu`
workunits expose events, switch/state groups, RTPCs, banks and bus objects as
named assets; `.wproj` contributes project metadata; `.bnk` is a header-level
BKHD view and `.wem` a RIFF/WAVE header view. FMOD `.fspro` project JSON is walked
for events, buses, VCAs and banks; `.strings` is a bounded NUL-separated path
list; `.bank` is an FSB5 header view. None of these decode audio payloads or
execute middleware. glTF `.gltf`/`.glb` inputs expose nodes, meshes, materials,
textures, images, animations and skins with containment and `uses_material` /
`uses_texture` / `uses_image` / `uses_mesh` / `uses_skin` edges; GLB content is
read from the JSON chunk only (no binary buffer decoding). External image and
buffer URIs are resolved relative to the source file and emitted as `references`
edges. Aseprite `.ase`/`.aseprite` reports layers and tags from chunk headers;
FBX reports only the header magic and version; Unity UPM `Packages/manifest.json`
and `packages-lock.json` become `upm_manifest`/`upm_lock` assets with
`upm_package` children. All pipeline inputs keep the same 16 MiB, two-second and
diagnostic bounds as other asset frontends, and `.shader` files still use the
ShaderLab asset parser.

Design and narrative inputs close the loop from planning to implementation.
Markdown design docs use a small, static marker dialect: `@id: REQ-X` declares a
requirement, and `@code: path#symbol`, `@asset:`, `@audio:`, `@video:`, `@art:`,
`@story: path#node`, `@value: table#key`, `@text: file#key` and `@link: REQ-Y`
(or `[[REQ-Y]]`)
become typed `design_*` edges; headings become `design_section` assets. Code
comments using `@design:`, `@spec:` or `@doc:` emit `implements` edges from the
source file to the referenced requirement, so a requirement can be traced to
code, assets, audio, video, art, story and config values in either direction.
Yarn and Ink scripts are parsed statically (nodes/knots/tags/options and jump
targets); scripts are never executed. Subtitles are text-only, and `.webm`/`.mkv`
inputs are header-level views. Repository docs such as README, CHANGELOG,
LICENSE and contribution guides are deliberately not indexed as design docs.

Localization inputs are read as static key/status inventories. gettext `.po`/
`.pot` entries use `msgctxt` (or `msgid`) as the key, `msgstr` as the target and
`#, fuzzy` as a status; the header entry supplies `Language:`. XLIFF 1.2
`trans-unit` and XLIFF 2.0 `unit` elements contribute id/source/target, with
`state="needs-translation"` or an empty target reported as untranslated. `.resx`
`data`/`value` pairs infer the locale from `Strings.<locale>.resx`; ARB
`@@locale` supplies it and `@`-prefixed metadata keys are ignored. Every entry
becomes a `localization_entry` asset with format, language, key, source, target
and status metadata, so untranslated and fuzzy strings can be counted and joined
to design requirements through `@text:`. No translator, fallback chain or
runtime locale resolution is modeled.

`codegraph233 languages` prints the canonical registry (id, backend, extensions,
strict/dynamic/asset mode). Projects can add custom extension mappings with
`extra_extensions`, e.g. `{"extra_extensions": {"gd2": "gdscript"}}`; keys are
lowercased, a leading dot is stripped and targets must be registered ids.
Custom mappings win over built-in extensions and are bounded to 64 entries.

Specify project roots on every MCP call. Roots are directories; excludes match
directory basenames or root-relative prefixes, **not globs**. Keep caches and
third-party inputs ignored. Parser/config changes invalidate old indexes; do
not alternate old/new indexers on one cache. `CODEGRAPH233_QUIET=1` selects one
currently allowed logical CPU and below-normal Windows priority;
`CODEGRAPH233_JOBS=1` bounds index workers. Power plans and unrelated processes
are unchanged.

Validation (an earlier 0.4.1-dev snapshot): 31 Rust tests passed on Windows
x64/Rust 1.99. Coverage includes owned
compile-only bytecode, descriptors/lambda/Unicode/long/double/switch/wide, MR JAR
resources, malformed refusal, source/import graphs, visible errors, unexecuted
PowerShell input, module extensions and incremental health. A local ordinary
launcher archive matched independent ASM: 27 classes, 930 methods, 105 fields,
12,441 direct invocations and 72 member hashes; 8,735 dynamic sites unresolved.
The 0.4.1-dev game-language work adds 14 targeted tests: five tree-sitter
extraction tests plus one Kotlin recovery test, four lexer-frontend tests, one
cross-language indexer integration test, two auto-language configuration tests
and one extension-mapping test covering GDScript, Kotlin, Swift, Objective-C,
Dart, Haxe, GML, HLSL, GLSL, Metal, WGSL and the `.usf`/`.ush`/`.cg`/`.gdshader`
project extensions. The Unreal/Godot work adds 16 more: four language-registry
tests, seven engine-asset tests (project/plugin JSON, INI paths, synthetic and
malformed uasset headers, Godot scene/project), one extension-override config
test, two indexer integration/mapping tests, one Unreal macro-masking source
test and one auto-update settings test. The art/audio pipeline work adds 8 more:
seven parser tests (Unity mixer, Wwise workunit/bank, FMOD project/strings/FSB5,
glTF, GLB, Aseprite, FBX/UPM) and one indexer integration test covering mixer
routing, Wwise/FMOD banks, glTF material edges and UPM packages. The
design/narrative work adds 7 more: six frontend tests (design markers and
links, Yarn nodes, Ink knots, subtitles, WebM headers, code annotations) and one
indexer integration test covering requirements, code `implements` edges, Yarn/Ink
jumps and subtitle/video assets. The localization work adds 5 more: four
frontend tests (PO status, XLIFF state/language, RESX/ARB, malformed input) and
one indexer integration test covering PO/XLIFF/RESX/ARB entries, untranslated
counts and a design `@text` link. The full suite
is 83 tests passing on Windows x64/Rust 1.99. A synthetic 10-file project with
`languages: ["auto"]`
was indexed end-to-end (zero parse diagnostics, all 22 frontends enabled), and
the existing `testdata/multi` fixture re-indexed without regressions in its C#,
Go, Rust, TypeScript, Lua or Unity-asset counts. A separate synthetic UE5/Godot
project (`.uproject`, `.uplugin`, `DefaultEngine.ini`, a header-level `.uasset`,
`.tscn`, `project.godot`, GDScript and C++ reflection macros) indexed with zero
parse diagnostics and the expected module/plugin/`/Game/`/main-scene/autoload
asset edges. A synthetic audio/art project (Unity mixer, Wwise workunit and
glTF scene) indexed with zero diagnostics, and `cg233_audio_graph` /
`cg233_art_graph` returned the expected assets and edges over MCP stdio. A
synthetic design project (Markdown requirement with `@code`/`@story` markers,
Yarn nodes, subtitle and WebM header) indexed with zero diagnostics, and
`cg233_design_graph` returned 9 assets / 12 edges including `implements`,
`design_code`, `design_story` and `jumps_to` over MCP stdio. A synthetic
localization project (PO, XLIFF, RESX and ARB plus a `@text` design marker)
indexed with zero diagnostics, and `cg233_localization_graph` returned 10 assets
/ 7 edges with 2 untranslated strings and the expected `design_text` link over
MCP stdio. A real
third-party project mixing these new languages was not used
for this validation. Raw third-party artifacts are not shipped. These checks do
not establish editor business-source recovery, application acceptance or support
for every language.

Follow-up validation: explicit project annotations, configuration invalidation,
original declaration prefixes, and real split C-linkage header members passed
targeted tests (two annotation tests, then five source-frontend tests). Two
intermediate signature assertions failed and were corrected before installation.
The final real SpineEditorX index includes the native codec C/header and complete
tooling directories: 1,987 files, zero parse diagnostics. Existing unrelated
passing tests were not repeatedly executed.

The real C/header export-name inventory also caught a pointer-return prototype
whose declarator node omitted its annotation prefix. Bounded ancestry now uses
the full declaration. The pointer-return test passed and both real files matched
all11 API names, original annotation prefixes and argument counts on parser
0.4.1-full5. The failed first inventory is retained separately.
