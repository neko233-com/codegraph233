# Source graph support in 0.4.1-dev

This is an unreleased local development version, separate from stable 0.4.0.
`languages: ["auto"]` enables C#, Go, Rust, TypeScript/JavaScript, Lua (and
Luau), Java, Kotlin, GDScript, Swift, Objective-C, Dart, Haxe, GameMaker GML,
C, C++, Python, PowerShell, HLSL/GLSL/Metal/WGSL source graphs and JVM
bytecode. Existing asset/configuration frontends remain controlled by project
profile/features.

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
project extensions. The full suite is 47 tests passing on Windows x64/Rust 1.99.
A synthetic 10-file project with `languages: ["auto"]` was indexed end-to-end
(zero parse diagnostics, all 22 frontends enabled), and the existing
`testdata/multi` fixture re-indexed without regressions in its C#, Go, Rust,
TypeScript, Lua or Unity-asset counts. A real third-party project mixing these
new languages was not used for this validation. Raw third-party artifacts are
not shipped. These checks do not establish editor business-source recovery,
application acceptance or support for every language.

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
