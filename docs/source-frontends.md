# Source graph support in 0.4.1-dev

This is an unreleased local development version, separate from stable 0.4.0.
`languages: ["auto"]` enables C#, Go, Rust, TypeScript/JavaScript, Lua, Java,
C, C++, Python, PowerShell and JVM bytecode. Existing asset/configuration
frontends remain controlled by project profile/features.

| Input | Implementation and scope |
| --- | --- |
| `.java` | Tree-sitter types, constructors, methods, fields, explicit imports and calls |
| `.c`, `.cc`, `.cpp`, `.cxx`, `.c++`, `.h`, `.hpp`, `.hh`, `.hxx` | Namespaces, types, functions/declarators and calls; `.h` uses C++ when enabled, otherwise C |
| `.py`, `.pyi` | Lexical classes/functions, calls, explicit/relative import aliases and `self` |
| `.ps1`, `.psm1`, `.psd1` | Declarations/commands; PowerShell 7 official parser is a bounded fallback on grammar recovery |
| `.mjs`, `.cjs`, `.mts`, `.cts` | Existing TS/JS parser and relative module normalization on original files |
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

Validation: 31 Rust tests passed on Windows x64/Rust 1.99. Coverage includes owned
compile-only bytecode, descriptors/lambda/Unicode/long/double/switch/wide, MR JAR
resources, malformed refusal, source/import graphs, visible errors, unexecuted
PowerShell input, module extensions and incremental health. A local ordinary
launcher archive matched independent ASM: 27 classes, 930 methods, 105 fields,
12,441 direct invocations and 72 member hashes; 8,735 dynamic sites unresolved.
Raw third-party artifacts are not shipped. These checks do not establish editor
business-source recovery, application acceptance or support for every language.

Follow-up validation: explicit project annotations, configuration invalidation,
original declaration prefixes, and real split C-linkage header members passed
targeted tests (two annotation tests, then five source-frontend tests). Two
intermediate signature assertions failed and were corrected before installation.
The final real SpineEditorX index includes the native codec C/header and complete
tooling directories: 1,987 files, zero parse diagnostics. Existing unrelated
passing tests were not repeatedly executed.
