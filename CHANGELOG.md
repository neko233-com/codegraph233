# Changelog

## 0.4.1-dev — unreleased

- Index design, narrative and media inputs: Markdown design docs with `@id`
  requirements and `@code`/`@asset`/`@audio`/`@video`/`@art`/`@story`/`@value`
  markers (plus `[[...]]` links) become a requirement graph; `@design:`/`@spec:`/
  `@doc:` code comments emit `implements` edges back from source files. Index
  Yarn and Ink scripts (nodes/knots/tags/options and jump targets), `.srt`/`.vtt`
  subtitles and `.webm`/`.mkv` EBML headers without executing scripts or decoding
  payloads. Add the `cg233_design_graph` MCP tool (26 total) and five registry
  entries.
- Index art and audio pipeline assets: Unity AudioMixer groups/snapshots/effects
  with `routes_to` routing; Wwise workunits/projects/bank/media headers; FMOD
  project/string-table/FSB5 banks; glTF/GLB nodes, meshes, materials, textures,
  images, animations and skins with material/texture/URI edges; Aseprite layers
  and tags; FBX headers; Unity UPM `manifest.json`/`packages-lock.json` packages.
  Add `cg233_audio_graph` and `cg233_art_graph` MCP tools (25 total).
- Index Unreal Engine 5 project assets: `.uproject`/`.uplugin` JSON modules,
  plugin dependencies and class redirects; `.ini` sections/keys with `/Game/`
  content paths and `/Script/` class references; `.uasset`/`.umap` header-level
  package magic, engine versions and name-table samples. Mask Unreal reflection
  macros (`UCLASS`, `UFUNCTION`, `UPROPERTY`, `GENERATED_BODY`, `*_API`, ...) for
  C++ parsing while preserving original byte/line positions.
- Index Godot 4 text scenes/projects: `.tscn`/`.tres` nodes, external/sub
  resources and `ExtResource` instances; `project.godot` main scene and autoload
  singletons.
- Add a canonical language registry (`codegraph233 languages`), `is_code_lang`
  dispatch and project-level `extra_extensions` mappings (bounded, validated,
  first match wins over built-in extensions).
- Extend auto-update controls: `update --enable`/`--disable`/`--interval HOURS`
  persist to `~/.codegraph233/settings.json`; `--status` reports the current
  auto-update state; interval is validated to 1–8760 hours.
- Add bounded tree-sitter GDScript/Kotlin/Swift/Objective-C/Dart and lexical
  Haxe/GameMaker GML/HLSL/GLSL/Metal/WGSL source graphs. Index `.gd`, `.kt`/`.kts`,
  `.swift`, `.m`/`.mm`, `.dart`, `.hx`, `.gml`, `.hlsl`/`.cginc`/`.compute`/
  `.usf`/`.ush`/`.cg`, `.glsl`/`.vert`/`.frag`/`.geom`/`.comp`/`.tesc`/`.tese`/
  `.gdshader`, `.metal`, `.wgsl` and
  `.luau` (via Lua); `languages: ["auto"]` now covers every implemented code
  frontend. Count call arity from raw source so string-only arguments resolve,
  and keep statically typed receivers strict while GDScript/GML stay dynamic.
  Kotlin grammar recovery is reported as diagnostics, not hidden.
- Add bounded, explicit project `c_annotations` for real native headers. Preserve
  annotation prefixes in original signatures and invalidate cached parse errors.

- Index `.mjs`, `.cjs`, `.mts` and `.cts` directly and normalize relative imports;
  content-copy views are no longer needed for those extensions.
- Add bounded tree-sitter Java/C/C++/Python/PowerShell source graphs, Python
  import aliases and PowerShell 7 parse-only recovery. Input scripts are never
  invoked. Preserve syntax diagnostics and reject guessed ambiguous receivers.
- Read ordinary CLASS/JAR inputs natively without a JVM; inventory archive member
  hashes, decode modified UTF-8 and select multi-release classes explicitly.
  Optional versioned ASM facts remain an independent import/oracle path.
- Expose persistent parse diagnostics, correct incremental unresolved counts and
  invalidate parser/configuration changes. Generic initialization enables auto
  languages; opt-in Windows quiet scheduling uses one CPU at below-normal priority.
- Reject partial/inconsistent fact streams and keep JVM binding descriptor-exact,
  including incremental discovery of previously missing declarations.
- This working-tree build is not a published stable release or a Java source
  recovery claim. Existing stable installations remain separately identifiable.

## 0.4.0 — 2026-11-05

**自更新、GitHub 镜像加速，并清理旧版本残留。**

### 新增

- `codegraph233 update`：检查 / 下载 / 替换当前二进制；`--check`、`--force`、`--mirror URL`、
  `--json`、`--status`。按平台选择 release 资产（Windows zip、Linux/macOS tar.gz）。
- 自动更新：`index` / `serve` 启动时后台检查（默认 24h 一次），发现新版本自动安装并提示重启客户端。
- 镜像加速：默认按 `ghfast.top` → `gh-proxy.com` → `ghproxy.net` → 直连回退；`mirror: "none"` 可禁用。
- 设置文件 `~/.codegraph233/settings.json` 与环境变量 `CODEGRAPH233_MIRROR` /
  `CODEGRAPH233_AUTO_UPDATE` / `CODEGRAPH233_NO_UPDATE`。
- 启动时自动清理 `.old-*` 与旧版本 `codegraph233-*.exe`。

### 变更

- 版本号统一从 `Cargo.toml` 读取（`codegraph233 version` 与 MCP `serverInfo`）。
- 安装/更新使用系统 `curl`，不新增 HTTP 依赖；下载后校验 release 资产大小。

## 0.3.1 — 2026-11-05

**补齐 SPEC §3.3 里承诺、但 Rust 版仍是“只有体积”的资产解析。**

### 新增

- 结构化 Unity YAML 资产（`.mat` / `.anim` / `.controller` / `.playable` / `.mixer` /
  `.asset` / `.spriteatlas`）：解析所有单行 `{fileID, guid}` 引用，产出
  `uses_script` / `uses_shader` / `uses_font` / `references` 边，字段名写入 `detail`。
- 材质：`shader_guid`、贴图槽数量；材质 → 贴图/材质 → shader 可直接反查。
- 动画片段：`sample_rate` / `duration_sec` / `events`。
- SpriteAtlas：`packed_sprites`。
- TMP 字体（`TMP_FontAsset`）：图集宽高、字符数、字形数、源字体 guid、AtlasPopulationMode。
- asmdef：程序集名、`references`、`autoReferenced`、`includePlatforms`。
- Spine：`.atlas` 页数 / 区域数 / 页尺寸；`.skel`（JSON 或二进制）版本，JSON 额外给出
  bones / slots / skins / animations 数量。
- TTF/OTF：新增字体名（name 表 nameID 4，按偏移读取，不整文件加载）。

### 变更

- 新增 `parser_version` 元数据：解析器行为变化时 `index` 自动重建，避免旧索引与新生代码不一致。
- schema v3：`asset_edges(src,dst,kind)` 唯一索引（打开旧库时一次性去重），解决每次 guid 物化重复插入边的问题；
  guid 解析成功后删除对应的原始 `guid:` 边，`cg233_asset_info` / `cg233_asset_refs` 不再出现一进一出两份同一关系。
- `cg233_project_health` 的 guid 覆盖率改读索引期写入的统计，不再依赖已被替换的原始边。

## 0.3.0 — 2026-11-05

**MCP 工具补齐、Lua 前端、增量重绑定、索引体积减半。**

### 新增

- MCP 工具从 15 个补齐到 23 个，覆盖 SPEC §4 全量契约：
  `cg233_overview`、`cg233_symbol_path`、`cg233_asset_tree`、`cg233_asset_stats`、
  `cg233_asset_orphans`、`cg233_shader_info`、`cg233_config_tables`、`cg233_config_readers`。
- `cg233_asset_tree`：prefab/scene 的 GameObject 层级、真实 parent、每个节点的
  `uses_script` / `instantiates` / `references` 边。
- `cg233_config_readers`：定位代码中引用某张配表的位置（文件名 / sheet / 表名），用于热重载探测。
- Lua 前端：`function a.b` / `function a:b` / `local function` / `local x = function` /
  表构造 / `require`；调用与成员引用；`--`、`--[[ ]]`、`[[ ]]` 正确跳过。
- `codegraph233 compact`：`ANALYZE` + `VACUUM`，回收增量索引产生的空闲页。
- `serve --project <name|root>` 与 `CODEGRAPH233_PROJECT`：全局 MCP 条目在无法从 cwd
  发现 `.codegraph233.json` 时回退到项目注册表。
- 跨文件边恢复置信度输出（`exact` / `type` / `name` / `name-multi`）。

### 变更

- **增量重绑定**：小 delta 只重算变更文件的 refs、指向其旧/新符号的 refs，以及名字命中新符号的
  未解析 refs；边按受影响符号集合删除重建。49,599 文件单文件改动 3.7s（原全局重绑定 36s），
  与 `--force` 全量重建逐边一致（自动化测试覆盖）。
- 无改动调用直接返回（< 10ms）。
- schema v2：refs 去掉 `src_key` / `receiver` / `root` 冗余列，置信度改整数，
  `namespace_hint` 与符号 namespace 相同时不落库，签名截断首行 200 字符；
  `index` 检测到旧 schema 自动重建，`status` 报告 `stale`。
- 团结工程（49,599 文件）DB：918MB → 394MB，compact 后 371.5MB；全量索引 20.2s。
- README / SPEC 同步：Lua 状态、P1 完成情况、23 个工具清单、compact 命令。

### 修复

- shader 关键字提取在 `multi_compile _ LOW HIGH` 这类“下划线占位”写法下丢关键字。
- prefab 层级树把同时挂在文件与父物体下的节点丢失父关系。
- MCP `cg233_projects` 的 `default` 在 Windows 路径分隔符不同时判断错误。
