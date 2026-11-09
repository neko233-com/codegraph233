# codegraph233

**AST GraphRAG for game projects — Rust 实现。**

索引 C# / Go / Rust / TypeScript / JavaScript / Lua / Java / C / C++ / Python / PowerShell / GDScript / Kotlin / Swift / Objective-C / Dart / Haxe / GameMaker GML / HLSL / GLSL / Metal / WGSL 源码、普通 CLASS/JAR、Unity 与 Unreal Engine 5 / Godot 工程资产、美术与音频管线资产（glTF / Aseprite / FBX / UPM、Unity AudioMixer / Wwise / FMOD）、策划文档 / 剧情脚本 / 字幕 / 视频头、本地化文本（PO / XLIFF / RESX / ARB / CSV）、Office 与 PDF 文档（DOCX / PPTX / PDF）、游戏资产与配表，通过 MCP 提供给 agent 查询。仓库已从 Go 完全迁移到 Rust（Go 代码已删除，历史基准见 [docs/benchmarks.md](docs/benchmarks.md)）。

## 能力（Rust 实现）

| 能力 | 状态 |
| --- | --- |
| C# 前端（namespace / 类型 / 成员 / 字段 / 属性 / enum / 方法 / 调用 / 构造 / 引用 / 继承 / 类型依赖） | ✅ |
| Go 前端（package / struct / interface / func / method / 跨包 receiver） | ✅ |
| Rust 前端（mod / struct / enum / trait / impl / fn / use 别名） | ✅ |
| TypeScript 前端（class / interface / enum / function / import 别名 / extends / implements） | ✅ |
| Lua 前端（模块函数 / `a.b` 与 `a:b` 方法 / local 函数与表 / require / 调用） | ✅ |
| Luau（`.luau`，复用 Lua 前端） | ✅ |
| Node 模块扩展名 `.mjs` / `.cjs` / `.mts` / `.cts`（沿用现有 TS/JS 静态解析） | ✅ 工作树开发版；不宣称完整 CommonJS 推断 |
| Java / C / C++ / Python / PowerShell 源码 AST | ✅ 0.4.1-dev；有界静态声明与引用，错误和动态调用保留；[边界](docs/source-frontends.md) |
| GDScript / Kotlin / Swift / Objective-C / Dart 源码 AST | ✅ 0.4.1-dev；tree-sitter 语法；类 / 方法 / 字段 / 枚举 / 构造 / 调用 / 继承；错误可见；[边界](docs/source-frontends.md) |
| Haxe / GameMaker GML 源码前端 | ✅ 0.4.1-dev；词法级类型 / 字段 / 方法 / 构造器 / 调用；GML 支持 constructor 与静态方法 |
| HLSL / GLSL / Metal / WGSL 源码前端 | ✅ 0.4.1-dev；函数 / struct / 调用；含 `.usf` / `.ush` / `.cg` / `.gdshader`；`.shader` 仍走资产解析器（properties / include / 关键字） |
| 普通 CLASS / JAR 原生读取、资源哈希与 JVM 描述符关系 | ✅ 0.4.1-dev；不需要 Java 运行时，不执行输入；[格式与边界](docs/jvm-facts.md)，不是反编译器 |
| Unreal Engine 5 工程资产（`.uproject` / `.uplugin` JSON 模块与插件、`.ini` 的 `/Game/` 与 `/Script/` 引用、`.uasset` / `.umap` 头级包信息） | ✅ 0.4.1-dev；uasset 只读文件头 / 版本 / 名称表样本，不解压属性、不解析蓝图逻辑；[边界](docs/source-frontends.md) |
| Godot 4 场景与工程（`.tscn` / `.tres` 节点、外部资源、`ExtResource` 实例、`project.godot` 主场景与 autoload） | ✅ 0.4.1-dev；文本格式静态解析，不启动 Godot |
| 语言注册表与扩展点（`codegraph233 languages`、`extra_extensions` 自定义扩展名映射） | ✅ 0.4.1-dev；22 个代码前端 + 11 个资产 / 引擎前端 + 12 个美术 / 音频管线前端 + 5 个设计 / 叙事前端 + 5 个本地化 / CSV 前端 + 8 个依赖清单前端 + 6 个 CI / 构建前端 + 4 个 Office / PDF 前端 + 6 个法律 / 出版 / 文本前端（共 79 项）统一登记 |
| 音频管线（Unity AudioMixer `.mixer` group/snapshot/effect、Wwise `.wwu` / `.wproj` / `.bnk` / `.wem`、FMOD `.fspro` / `.strings` / `.bank`） | ✅ 0.4.1-dev；文本工程解析 + BKHD / FSB5 / RIFF 头级信息；不解析音频负载 |
| 美术管线（glTF `.gltf` / `.glb` 节点 / 网格 / 材质 / 贴图 / 动画 / 蒙皮、Aseprite `.ase` / `.aseprite` 图层与标签、FBX 头、UPM `Packages/manifest.json` / `packages-lock.json`） | ✅ 0.4.1-dev；含 `uses_material` / `uses_texture` / `references` 等资产边；不解码图片或音频负载 |
| 策划 / 叙事 / 媒体（Markdown 策划案的 `@id` / `@code` / `@asset` / `@audio` / `@video` / `@art` / `@story` / `@value` / `@text` 标记与 `[[...]]` 关联、Yarn / Ink 剧情脚本、`.srt` / `.vtt` 字幕、`.webm` / `.mkv` 头） | ✅ 0.4.1-dev；代码侧 `@design:` / `@spec:` / `@doc:` 注释生成 `implements` 边，形成策划 → 代码 → 资产全链路；不执行脚本 |
| 本地化 / i18n（gettext `.po` / `.pot`、XLIFF `.xliff` / `.xlf`、.NET `.resx`、Flutter `.arb`、Localization / UE `.csv`） | ✅ 0.4.1-dev；每条文案的 key / 原文 / 译文 / translated / fuzzy / untranslated 状态；`@text:` 策划标记可关联到文案文件；CSV 按 `Key` + 可选 `SourceString` + 语言列展开 |
| 本地化 QA 与代码→音频事件绑定（`cg233_localization_report`、`cg233_audio_events`） | ✅ 0.4.1-dev；聚合占位符不匹配 / 超长 / 未翻译 / 缺失译文 / 重复 key；扫描 Wwise `PostEvent`、FMOD `PlayOneShot` 等调用点并关联事件资产，输出按事件聚合、控制 token |
| 关系图扩展（npm / Cargo / Go / Python / vcpkg / Conan / NuGet / Maven 依赖清单、Markdown 文档链接、`.meta` importer → 资产边、`link_rules` 自定义路径规则） | ✅ 0.4.1-dev；依赖 `depends_on`、文档 `doc_link`、导入 `imports`、自定义 `kind`；规则最多 32 条，按 `{stem}` / `{name}` 等模板生成目标 |
| CI / 构建管线（Jenkinsfile、`.github/workflows/*.yml`、`.gitlab-ci.yml`、`azure-pipelines.yml`、CircleCI、Makefile） | ✅ 0.4.1-dev；行级静态解析，不执行任何流水线；stage / job / step / target 与 `uses_action` / `uses_stage` / `runs` / `depends_on` 关系，`cg233_ci_graph` 查询 |
| Office 与 PDF（DOCX 标题 / 段落 / 表格与 `@code` / `[[REQ]]` 标记、PPTX 幻灯片、PDF 页数 / Info / FlateDecode 文本、旧版 DOC/PPT/XLS 识别提示） | ✅ 0.4.1-dev；ZIP/XML 与 PDF 流静态读取，不执行宏、不做 OCR 或 CMap 排版还原；图表、批注、嵌入对象不索引；`cg233_document_graph` 查询；可用于软著、版号、专利、法务材料与代码 / 需求的关联 |
| 法律 / 专利 / 出版文本（TXT、RTF、ODT、EPUB、HTML、LaTeX） | ✅ 0.4.1-dev；专利权利要求（`CLAIMS` / 权利要求书，编号 claim 与 `claims` 边）、法务条款（`第 N 条` / `Article N`，`legal_clause`）、专利号（CN/US/WO/EP/JP/KR）、章节、LaTeX 引用；统一走标记者 / 法务扫描，`cg233_document_graph` 查询 |
| prefab / scene（GameObject 层级、m_Script、嵌套 prefab、guid 引用） | ✅ |
| shader（Shader / property / include / multi_compile 关键字 / HLSL 函数） | ✅ |
| 媒体资产（图片宽高、音频采样率/码率/时长、视频时长、字体名与字形数、Spine atlas/skel） | ✅ |
| 配置表（xlsx / csv / tsv / json，表头与行数、读取点探测） | ✅ |
| 结构化资产（材质/动画/控制器/时间轴/混音器/SpriteAtlas/TMP 字体/asmdef）的 guid 引用 | ✅ |
| guid 映射（编辑器导出 TSV/CSV/JSON + 明文 `.meta` 补充 + 资产边物化） | ✅ |
| 增量索引（mtime/size）、prune、并行绑定 | ✅ |
| 增量重绑定（小 delta 只重算相关边，结果与全量重建一致） | ✅ |
| 冷热分离与持续开发刷新（`hot_days` 热/冷文件分层、`heatmap` 统计、`watch` 轮询增量索引、`cg233_heatmap` / `cg233_refresh`） | ✅ |
| CLI：`init` / `index` / `watch` / `status` / `heatmap` / `compact` / `register` / `projects` / `languages` / `install-mcp` | ✅ |
| MCP stdio + 34 个工具 | ✅ |
| 多项目（注册表 `~/.codegraph233/projects.json`、工具 `project` 参数、`cg233_projects`） | ✅ |

## 安装（全局一次）

```powershell
./install.ps1                 # cargo install --path . + 注册 MCP（默认 global）
./install.ps1 -Scope project  # 可选：给单个项目写带 --root 的兜底条目
```

```sh
./install.sh                  # macOS / Linux / WSL
```

手动：

```sh
cargo install --path . --force
codegraph233 version
```

开发验证使用 Rust 1.99；尚未单独验证最低 Rust 版本。`rusqlite` 使用 bundled 原生 SQLite，首次编译约 1–2 分钟。

## 快速开始

```sh
cd <game-project>
codegraph233 init        # 生成 .codegraph233.json（自动识别团结/Unity）
codegraph233 index       # 全量索引，之后自动增量
codegraph233 status      # 统计
codegraph233 languages   # 已登记的语言 / 引擎前端与扩展名（--json 可机读）
codegraph233 serve       # MCP stdio（客户端会自动拉起）
```

## 性能（Rust）

本仓实测（49,599 文件，含 C#/Go/资产/配表/`.meta`）：

| 指标 | 数值 |
| --- | --- |
| 全量索引（Rust） | 见 [docs/benchmarks.md](docs/benchmarks.md) |
| 无改动增量 | < 10 ms（不解析、不重绑定） |
| 单文件增量 | 只重算该文件相关边（`rebind=incremental`） |
| 产物 | 356k symbols / 2.17M refs / 990k edges / 40,116 assets / 27,716 guid |
| 无 GC | ✅（Rust 无 GC，Go 版 GC 本身也只占 0% CPU） |

RSS 仍由 SQLite 页缓存决定（~2.4GB 量级），换语言不会改变这一点。

## 更新与镜像加速

```sh
codegraph233 update --check     # 只检查：当前版本 vs GitHub 最新 release
codegraph233 update             # 下载并替换当前二进制
codegraph233 update --force     # 即使版本相同也重装
codegraph233 update --status    # 查看自动更新设置和 mirror
codegraph233 update --check --json
codegraph233 update --mirror https://ghfast.top/   # 换镜像并持久化
codegraph233 update --enable --interval 12         # 开启自动更新，12 小时一次
codegraph233 update --disable                      # 关闭自动更新
```

* 自动更新默认开启：`index` / `serve` 启动时在后台检查，24 小时最多一次；发现新版本会下载、
  替换二进制并提示重启 MCP 客户端。`--no-update` 可跳过本次检查。
* `--enable` / `--disable` / `--interval HOURS` / `--mirror URL` 会持久化到设置文件
  （interval 取值 1–8760 小时），`--status` 立即回显当前设置。
* 设置文件：`~/.codegraph233/settings.json`（`auto_update`、`interval_hours`、`mirror`、`mirrors`、
  `last_check`、`last_version`）。可直接编辑。
* 环境变量：`CODEGRAPH233_MIRROR`（覆盖镜像）、`CODEGRAPH233_AUTO_UPDATE=0`（关闭自动更新）、
  `CODEGRAPH233_NO_UPDATE=1`（本次不检查）。
* 默认镜像顺序：`ghfast.top` → `gh-proxy.com` → `ghproxy.net` → GitHub 直连；任一成功即停止。
  `mirror: "none"` 表示只用直连。下载走系统 `curl`，不引入新的 HTTP 依赖。
* Windows 下正在运行的 exe 无法覆盖，更新采用“重命名旧文件 → 写入新文件”，因此更新后需重启
  MCP 客户端；遗留的 `.old-*` 与旧版本 exe 会在下次启动时自动清理。
* GitHub Actions：`.github/workflows/ci.yml` 在 Ubuntu / Windows / macOS 上跑
  `cargo test --locked` 与 release 构建；推送 `v*` tag 时 `.github/workflows/release.yml`
  自动打包三平台产物（zip / tar.gz）并创建 GitHub Release。

## 配置

`.codegraph233.json`（提交）：

```json
{
  "version": 1,
  "strategy": "tuanjie",
  "profile": "full",
  "roots": [
    { "key": "client", "path": ".", "role": "frontend" },
    { "key": "server", "path": "../server-project-sf-go", "role": "backend" }
  ],
  "languages": ["csharp"],
  "exclude": ["Assets/Plugins"]
}
```

* `profile`：`full` / `game` / `code` / `minimal`；`features` 可按项覆盖。
* `languages` 省略默认 `["csharp"]`；`["auto"]` 表示全部已实现前端（`CODE_LANGUAGES`：csharp / go / rust / typescript / lua / jvm / java / c / cpp / python / powershell / gdscript / kotlin / swift / objc / dart / haxe / gml / hlsl / glsl / metal / wgsl）。`codegraph233 languages` 列出全部登记项及其扩展名；管线与引擎资产 id（gltf / aseprite / fbx / upm_manifest / unity_mixer / wwise_* / fmod_* 等）同样可用于 `extra_extensions`。
* `extra_extensions`：把任意扩展名映射到已登记的前端，例如
  `{"extra_extensions": {"gd2": "gdscript", "mm2": "objc", "scene": "godot_scene"}}`。
  键会自动小写并去掉前导 `.`，目标必须是 `codegraph233 languages` 中存在的 id；
  自定义映射优先于内置扩展名，最多 64 条。
* `link_rules`：声明式路径关系，不写代码也能把任意文件连成图。每条规则按
  `include` glob 匹配，`dst` 支持 `{stem}` / `{name}` / `{ext}` / `{dir}` / `{path}` / `{root}`，
  生成边的 `kind` 与可选 `detail` 自定义，例如：
  `{"link_rules": [{"include": "Audio/*.wav", "kind": "sfx_for", "dst": "audio_event:{stem}"}]}`。
  最多 32 条；目标带 scheme（如 `design:`）时原样保留，否则按 `asset:<root>:<dst>` 关联。
  文件未命中任何规则时不会产生额外汇总开销。
* 索引产物：`.codegraph233/index.db`（gitignore，可重建）。schema 版本变化时 `index` 会自动重建，`status` 会提示 `stale schema`。
* `codegraph233 compact`：`ANALYZE` + `VACUUM`，回收增量索引产生的空闲页。

## MCP 工具（34）

代码图：`cg233_status` / `cg233_index` / `cg233_overview` / `cg233_projects` / `cg233_project_health` /
`cg233_find_symbol` / `cg233_get_symbol` / `cg233_symbol_path` / `cg233_file_outline` /
`cg233_find_referencing_symbols` / `cg233_call_graph` / `cg233_impact` / `cg233_search_pattern`

资产图：`cg233_asset_search` / `cg233_asset_info` / `cg233_asset_refs` / `cg233_asset_tree` /
`cg233_asset_stats` / `cg233_asset_orphans` / `cg233_guid_lookup`

管线图：`cg233_audio_graph`（AudioMixer / Wwise / FMOD 路由）/ `cg233_art_graph`（glTF / Aseprite / FBX / UPM）/ `cg233_design_graph`（策划案 / 剧情 / 代码标记）/ `cg233_localization_graph`（PO / XLIFF / RESX / ARB 文案状态）/ `cg233_localization_report`（占位符 / 超长 / 未翻译 / 重复 key 聚合 QA）/ `cg233_audio_events`（代码 → Wwise / FMOD 事件绑定）/ `cg233_dependencies`（npm / Cargo / Go / Python / vcpkg / Conan / NuGet / Maven 依赖聚合）/ `cg233_ci_graph`（Jenkins / GitHub Actions / GitLab / Azure / CircleCI / Make 构建图）/ `cg233_document_graph`（DOCX / PPTX / PDF / 法律 / 专利 / 纯文本、标记与需求关联）/ `cg233_heatmap`（热 / 冷文件分层）/ `cg233_refresh`（mtime 增量刷新）

游戏数据：`cg233_shader_info` / `cg233_config_tables` / `cg233_config_readers`

所有工具都接受可选 `project`（项目名 / 根路径 / 路径片段）。

```sh
codegraph233 register --root <项目>   # 注册到全局表
codegraph233 projects                 # 列出 + 索引状态
```

## 开发

```sh
cargo build --release
cargo test
```

MIT.
