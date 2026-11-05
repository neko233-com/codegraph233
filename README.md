# game-project-graph

**AST GraphRAG MCP server for game projects.** One index for the client (Unity / Tuanjie / UE-ready) and the backend (Go), plus the game-specific data that generic code-graph tools cannot see: **prefabs/scenes, shaders and business config tables**.

It is inspired by Serena's symbol-level tooling and CodeGraph's graph queries, but targets game repositories:

| Capability | codegraph / Serena | game-project-graph |
| --- | --- | --- |
| C# symbols / calls / inheritance | partial | ✅ cross-file, overload-aware |
| Go symbols / calls / interface embedding | partial | ✅ standard-library AST |
| Multi-root (frontend + backend, backend optional) | ❌ | ✅ `.game-project-graph.json` |
| Engine auto-detection (Unity / Tuanjie / UE) | ❌ | ✅ reads `ProjectVersion.txt`, `.uproject` |
| Prefab / scene hierarchy + script & asset refs | ❌ | ✅ Unity/Tuanjie YAML |
| ShaderLab properties / passes / includes / keywords | ❌ | ✅ |
| Business config tables (xlsx/csv/tsv/json, auto format) | ❌ | ✅ + hot-reload detection |
| Change impact / call paths / reference confidence | partial | ✅ |

## Install (one command, global)

Windows (PowerShell 7+):

```powershell
./install.ps1
```

Linux / macOS / WSL:

```sh
./install.sh
```

Both scripts:

1. verify Go (1.22+, 1.27 recommended),
2. `go install` the binary into `$(go env GOPATH)/bin` (i.e. global),
3. make sure that directory is on `PATH`,
4. optionally register the MCP server in the current project.

Options: `-Source <local checkout>`, `-Ref <tag|branch>`, `-NoMcp`, `-Clients codex,cursor,vscode,air,cowork`.

Manual install:

```sh
go install github.com/neko233-com/game-project-graph/cmd/game-project-graph@main
```

## Quick start

```sh
cd <your-game-project>
game-project-graph init          # writes .game-project-graph.json, auto-detects engine/languages
game-project-graph index         # full index (incremental afterwards)
game-project-graph status        # detection + counts
game-project-graph install-mcp   # add the MCP entry to Codex/Cursor/VS Code/Air/Cowork
```

`init` appends `/.game-project-graph/` to `.gitignore`; the index is local state and must not be committed.

## Config

`.game-project-graph.json` (committed):

```json
{
  "version": 1,
  "strategy": "auto",
  "roots": [
    { "key": "client", "path": ".", "role": "frontend" },
    { "key": "server", "path": "../server-project-sf-go", "role": "backend" }
  ],
  "languages": ["csharp", "go"],
  "config_dirs": ["Team-Resources/BusinessConfig"],
  "exclude": ["Assets/Plugins", "_server"]
}
```

* `strategy`: `auto` | `unity` | `tuanjie` | `ue` | `generic`. `auto` reads `ProjectSettings/ProjectVersion.txt`; a Tuanjie build has a `t` patch level (`2022.3.62t17`), Unity has `f`/`a`/`b`/`p`.
* `roots`: any number. Frontend/backend/shared are conventions, not requirements; a missing path is reported as `missing` and does not fail the run.
* `config_dirs`: extra config-table roots (absolute or relative to the config file).
* `exclude`: directory names or root-relative prefixes.

Index location: `<project>/.game-project-graph/index.db` (SQLite, WAL).

## MCP tools

| Tool | Purpose |
| --- | --- |
| `gpg_status` | detection, roots, languages, index counts |
| `gpg_index` | build/update (incremental; `force`, `jobs`) |
| `gpg_overview` | strategy, histogram, top namespaces, asset kinds, config/hot-reload summary |
| `gpg_find_symbol` | find types/methods/fields/shader functions by name or qualname |
| `gpg_get_symbol` | symbol + numbered source body |
| `gpg_file_outline` | declarations of a file |
| `gpg_find_referencing_symbols` | incoming references (Serena parity) |
| `gpg_call_graph` | callers/callees, multi-hop |
| `gpg_symbol_path` | shortest call path between two symbols |
| `gpg_impact` | change-impact set and affected files |
| `gpg_search_pattern` | regex search with context (for unmodelled constructs) |
| `gpg_asset_search` | prefab/scene objects, shaders, config tables |
| `gpg_asset_tree` | prefab/scene GameObject hierarchy |
| `gpg_asset_refs` | instantiates / uses_script / references / includes |
| `gpg_shader_info` | properties, passes, includes, keywords |
| `gpg_config_tables` | detected tables with headers/rows/source |
| `gpg_config_readers` | config read sites + hot-reload capability |

All tools return JSON. Symbol ids are stable within one index build.

## Strategies

**tuanjie / unity.** C# is indexed with a tolerant declaration scanner (namespaces, nested/partial types, methods, properties, fields, events, enum members, calls, constructs, inheritance). Prefabs and scenes are parsed from their YAML: GameObject hierarchy, `m_Script` GUIDs, nested prefabs, asset references.

> Tuanjie stores encrypted GUIDs in `.meta`, so a prefab GUID cannot be mapped back to an asset path without the editor. The graph keeps GUID relationships (and therefore duplicate/shared-reference analysis) but does not resolve GUID→path. Unity projects have plaintext GUIDs and can be resolved later.

**ue (reserved).** Projects are recognized (`.uproject` / `Content/*.uasset`) and reported, but C++/Blueprint/asset graphs are not implemented yet.

**generic.** Any repo with C#/Go works; prefab/shader/config stages simply find nothing.

## Config tables

Format is auto-detected, not hardcoded:

* `.xlsx` — one table per sheet (headers, row count), streamed, column-capped to avoid dirty-column blowups;
* `.csv` / `.tsv` — header + row count;
* `.json` — array/object shape (rows, header keys).

`gpg_config_readers` reports code that reads config (`GetDataById`, `ConfigManager`, `GetConfig`, …) and whether a hot-reload implementation exists (`ConfigHotReloadManager`, `ReloadConfig`, `Config233AutoExportWatcher`, …).

## Notes / limits

* The C# front-end is a scanner, not a compiler: it resolves local variables, parameters, fields and obvious static receivers; remaining references are reported with `confidence: name` / `name-multi`.
* Go uses `go/parser`; cross-package resolution goes through import paths. Interface satisfaction is represented by `embeds`/`implements` edges from declarations, not by full method-set analysis.
* Large generated folders must be excluded; indexing is CPU-bound and parallel (`--jobs`).
* `game-project-graph index --force` re-parses everything and is the escape hatch when the graph looks stale.

## Development

```sh
go test ./...
go vet ./...
go build ./cmd/game-project-graph
```

MIT licensed.
