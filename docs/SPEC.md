# codegraph233 规范

> 本文整理「为什么要做 codegraph233、它应该是什么、边界在哪」。实现细节变化时先改本文件，再改代码。
> 最后更新：2026-11-05

## 1. 目的与定位

**定位一句话：Serena 的符号级工具 + Graphify 的图检索能力 + 对游戏工程的深度支持。**

通用代码图工具（codegraph / graphify / Serena）解决的是「读懂代码」，但它们不懂游戏项目里真正占工作量的一半东西：prefab 层级、shader 结构、业务配表，以及「客户端 + 服务端两个仓库」这种工程形态。反过来，游戏项目的 agent 又最需要这些上下文。

因此 codegraph233 的三个能力域必须同时成立：

1. **代码语义图**：跨文件、跨工程的符号、调用、引用、继承、构造关系，带置信度。
2. **游戏数据图**：prefab / scene 的 GameObject 层级与脚本、资源引用；ShaderLab 的 property / pass / include / keyword；业务配表的结构与读取点。
3. **工程识别与策略**：自动判断引擎（团结 / Unity / UE）、语言、配置目录，并按策略决定索引范围。

不是「又一个 grep 加速器」，而是「让 agent 在游戏工程里一次问对」。

## 2. 设计原则

| 原则 | 含义 |
| --- | --- |
| 单二进制、零依赖 | 纯 Go（SQLite 用 modernc），无 cgo；`go install` 即完成安装，ps1/sh 一键全局安装 |
| 只读优先 | 不提供 `replace_symbol` / `write_file` 这类改写工具；改写交给编辑器与人工，避免 agent 误改资产 |
| 策略驱动、可扩展 | 引擎策略、语言策略、资产策略三层分离；新增语言/引擎只加策略与前端，不改查询与 MCP 契约 |
| 索引是一等公民 | 索引是本地可再生状态（`gitignore`），增量更新；损坏可 `--force` 重建 |
| 召回优先、精度显式 | 宁可多解析，也要把 `exact` / `type` / `name` / `name-multi` 置信度返回给调用方，由 agent 判断 |
| 不追求编译级正确 | 不做 LSP 语义分析；接受少量误报，换取 2 万文件级的索引速度与零依赖部署 |

## 3. 策略矩阵

### 3.1 语言

| 语言 | 级别 | 状态 | 说明 |
| --- | --- | --- | --- |
| C# | **default** | 已实现 | 一级支持：namespace / 嵌套与 partial 类型 / 方法 / 属性 / 字段 / 事件 / 枚举成员；调用、构造、引用、继承；局部变量与字段类型推断；Unity/Tuanjie 资产联动 |
| Go | optional | 已实现 | 后端场景：包路径跨包解析、receiver/字段类型推断、结构体嵌入（`embeds`）。默认**不开启**，需在 `languages` 显式加入 |
| Rust | optional | 已实现 | `mod/struct/enum/trait/impl/fn/type/const`；`use` 别名展开、`crate::/self::/super::` 路径、impl 方法归属、trait 实现、`let x: T`/`T::new()` 类型推断 |
| TypeScript / JavaScript | optional | 已实现 | `class/interface/type/enum/function`、构造与箭头函数、`import` 别名（相对路径解析到模块）、`extends/implements`、`new`/成员调用 |
| Lua | optional | 已实现 | `function a.b` / `function a:b` / `local function` / `local x = function` / 表构造 / `require`；`--` 与 `--[[ ]]` 注释、`[[ ]]` 长字符串；块匹配支持 if/for/while/repeat/standalone do |

### 3.2 引擎 / 工程策略

| 策略 | 级别 | 状态 | 判据 |
| --- | --- | --- | --- |
| `tuanjie` | default | 已实现 | `ProjectSettings/ProjectVersion.txt` 的版本带 `t` 补丁号（如 `2022.3.62t17`）；`.meta` guid 为加密形式 |
| `unity` | default | 已实现 | 同文件版本为 `f/a/b/p` 补丁号；`.meta` guid 明文，可按 guid 反查资产路径 |
| `ue` | planned | **仅识别** | `.uproject` 或 `Content/*.uasset`；C++/蓝图/资产图未实现 |
| `generic` | fallback | 已实现 | 任何 C#/Go 仓库；资产阶段自然为空 |

### 3.3 资产

| 资产 | 状态 | 产出 |
| --- | --- | --- |
| prefab / scene | 已实现 | GameObject 节点、父子 `contains`、`m_Script` → `uses_script`、嵌套 prefab → `instantiates`、资源 guid → `references` |
| shader | 已实现 | shader / pass / property 节点、`includes`、`multi_compile` 关键字、HLSL 函数符号 |
| 图片（png/jpg/tga/psd/exr/webp/bmp/gif/hdr/dds） | 已实现 | 宽度/高度/体积（头部解析，不读整文件） |
| 音频（wav/mp3/ogg/flac/m4a/aac/aif/mod…） | 已实现 | 采样率、声道、时长、码率（头部；OGG 读尾部 granule） |
| 视频（mp4/mov/webm/mkv/avi） | 已实现 | 时长、分辨率（mvhd/tkhd；moov 在尾部时读 tail） |
| TTF/OTF/TTC 字体 | 已实现 | 字体名（name 表 nameID 4）、字形数（maxp 表），按偏移 ReadAt，不读整文件 |
| TMP 字体（`TMP_FontAsset`） | 已实现 | 图集宽高、字符数、字形数、源字体 guid、AtlasPopulationMode |
| Spine（.skel/.atlas） | 已实现 | atlas 页数/区域数/页尺寸；.skel（JSON）版本与 bones/slots/skins/animations 数量，二进制版本号 |
| 材质/动画/控制器/时间轴/混音器 | 已实现 | 解析全部 `{fileID, guid}` 引用为 `uses_script` / `uses_shader` / `uses_font` / `references` 边（detail=字段名）；材质附 `shader_guid`、贴图槽数；动画附采样率/时长/事件数；SpriteAtlas 附 packed_sprites |
| 资产→资产直接边 | 已实现 | 索引结束把 guid 端点物化为 `asset:<root>:<path>`，可直接回答"这个材质用了哪些贴图/这个 prefab 实例化了哪些 prefab" |
| 孤儿资产 / 健康报告 | 已实现 | `cg233_asset_orphans`（未被引用的资源，体积排序）、`cg233_project_health`（覆盖率/孤儿/DB 体积/最大资产） |
| asmdef | 已实现 | 程序集名、references、autoReferenced、includePlatforms |
| 配表 | 已实现 | xlsx（每个 sheet 一张表，**自动挑字段名行**）、csv/tsv、json；表头/行数/列数/说明行；读取点与热重载探测 |
| guid → 路径 | 已实现 | 编辑器导出 TSV（100% 资产，本仓 27,412 条）为主；明文 `.meta`（32 位）自动补充；本仓引用覆盖率 96.7% |

## 3.5 能力开关与性能档

* `profile`: `full` / `game` / `code` / `minimal`；`features` 八项（code、type_deps、prefabs、shaders、assets、meta、config_tables、guid_map）可逐项覆盖。
* `languages: ["auto"]` 按实际检测启用；显式列表优先。
* 写入档：默认快速档（`journal_mode=OFF`、`synchronous=OFF`、mmap、128MB cache），索引可重建，崩溃只丢索引；`index --safe` 使用 WAL 持久档。
* 写库模式：`single`（默认，整轮一个事务，实测 64.9s）> `batch`（每 300 文件提交，69.4s）> `none`（无事务，155.1s）。无事务不会更快。
* 调度：`index --low-priority`（Windows BELOW_NORMAL）避免抢编辑器 CPU；索引期 GC 目标 250。
* 多项目：`~/.codegraph233/projects.json` 注册表 + 每项目独立 DB（懒加载），工具统一接受 `project` 选择器。
* 绑定阶段：分页读取、按 CPU 并行解析、只写目标发生变化的引用。
* 增量重绑定：小 delta（≤2,000 文件且 ≤1/3 总数）只重绑定变更文件的 refs、指向其旧/新符号的 refs、以及名字命中新符号的未解析 refs；边按受影响符号集合删除后重建。实测 49,599 文件单文件改动 3.7s（原全局重绑定 36s），结果与 `--force` 全量重建逐边一致（`cargo test` 覆盖）。\n* 无改动调用直接返回（<10ms），不再做全局重绑定。\n* 空间：refs 去掉 `src_key`/`receiver`/`root` 冗余列、置信度改小整数、签名截断首行 200 字符、`namespace_hint` 与符号 namespace 相同时不落库；团结工程（49,599 文件）实测 918MB → 394MB，`codegraph233 compact`（ANALYZE + VACUUM）后 371.5MB。
* 非目标补充：不追求"崩溃后索引完好"，只保证"索引可无副作用重建"；资产元信息不替代 AssetDatabase 的导入设置。

## 3.6 技术选型：Go vs Rust（实测）

* C# 解析（33,755 文件）：Go 447ms vs Rust 168ms（Rust 侧抽取更简，实际 1.5–2×）。
* 写库（100 万 refs + 80 万 edges，同 schema）：Go 9,538ms vs Rust **1,368ms（7.0×）**。
* 索引期 GC（Go）：141 次、CPU 0%、STW 49.6ms、堆 5–6MB —— GC 不是瓶颈。
* 差异根因是 SQLite 实现：Go 用 `modernc`（纯 Go），Rust 用 `rusqlite`（原生 C）。
* 结论：**换 Rust 的收益来自存储引擎**，预期全量 65s → 30–35s。RSS 量级不变。
* 路线：`rust/` 阶段 0 基准已完成；阶段 1 索引内核（C# + store + index/status）→ 阶段 2 其余前端与资产 → 阶段 3 MCP。

## 4. MCP 契约## 4. MCP 契约

- 传输：stdio，JSON-RPC 2.0；server name `codegraph233`。
- 工具前缀：`cg233_`。
- 所有工具返回 JSON；空结果返回 `[]` / `[]`，不返回 `null`。
- 符号 id 在单次索引内稳定；重建索引后可能变化，调用方不应跨索引持久化 id。
- 置信度语义：
  - `exact`：限定名或同类型/同命名空间直接命中；
  - `type`：通过 receiver 类型解析命中；
  - `name`：全局唯一名字兜底；
  - `name-multi`：同名多候选，已按 root/lang/arity 择优，仍需人工确认。

工具清单（详见 README）：

`cg233_status` / `cg233_index` / `cg233_overview` / `cg233_find_symbol` / `cg233_get_symbol` / `cg233_file_outline` / `cg233_find_referencing_symbols` / `cg233_call_graph` / `cg233_symbol_path` / `cg233_impact` / `cg233_search_pattern` / `cg233_asset_search` / `cg233_asset_tree` / `cg233_asset_refs` / `cg233_asset_stats` / `cg233_asset_info` / `cg233_asset_orphans` / `cg233_project_health` / `cg233_guid_lookup` / `cg233_shader_info` / `cg233_config_tables` / `cg233_config_readers`

## 5. 项目接入与分发

- 项目配置文件：`.codegraph233.json`（提交到仓库）。
- 索引产物：`.codegraph233/`（必须 gitignore，可随时删除重建）。
- 多根目录：`roots[]`，`role` 只是约定（frontend / backend / shared / assets），后端可空缺。
- 一键安装：`install.ps1` / `install.sh` → `go install` 到 `$(go env GOPATH)/bin` 并补 PATH。
- MCP 注册：`codegraph233 install-mcp --scope project|global|both`。
  - `project`：写入项目内各客户端配置，条目带 `serve --root <项目>`，锁定本项目；
  - `global`：写入用户级配置（`~/.codex`、`~/.cursor`、`~/.air`、`~/.codely-cli`、`~/.workbuddy`、VS Code 用户目录），条目为 `serve`，按客户端工作目录自动发现 `.codegraph233.json`；
  - `both`：两处都写，全局兜底 + 项目精确。

## 6. 非目标（明确不做）

1. 不做编译级类型检查或语法诊断（不替代 IDE / Roslyn）；资产元信息是头部/文本解析，不替代 AssetDatabase 的导入设置。
2. 不做代码改写、重命名、批量重构（不做 Serena 的编辑类工具）。
3. 不做 UE 的 C++ / 蓝图 / 资产图（策略位保留）。
4. 不尝试解密团结引擎的 `.meta` guid（需编辑器桥；当前保留 guid 关系，不做 guid→路径）。
5. 不做远端服务、多用户协作、权限体系（本地 stdio 单机工具）。
6. 不为「改表现」或「提帧」类目的服务（那是性能工具的事）。

## 7. 验收标准

| 维度 | 标准 |
| --- | --- |
| 覆盖率 | 抽样 100 个 C# 类型/方法：声明召回 ≥ 95%；调用边召回 ≥ 80%（含 `name` 级） |
| 索引速度 | 4.8 万文件全量 ≤ 90 秒（实测 20.2s，49,599 文件）；无改动增量 < 10ms；单文件增量 3.7s |
| 查询延迟 | 单次 `find_symbol` / `file_outline` < 200ms（30 万符号级索引） |
| 内存 | 全量重建峰值 ≤ 4GB；稳态查询 < 500MB |
| 稳定性 | 单文件解析失败不影响整批；索引中断后可增量续跑 |
| 可解释性 | 每条跨文件边必须带置信度；未解析引用可统计 |
| 资产关联 | 提供编辑器导出时，guid→资产解析覆盖率 ≥ 95%（本仓 96.7%）；无编辑器导出时明确报告覆盖率 |

## 8. 演进路线

**P0（已完成，2026-11-05）**
- C# / Go / Rust / TypeScript / Lua 语义图、prefab/scene、shader、配表、多根、引擎识别、SQLite 增量、一键安装、7 客户端接入（项目级 / 全局）。

**P1（已完成，2026-11-05）**
- 增量重绑定：小 delta 只重绑定受影响的 refs/edges，单文件改动 3.7s（原 36s）；与全量重建逐边一致（自动化测试）。
- 索引体积：refs 去冗余列 + 置信度整数化 + 签名截断 + 冗余 namespace 不落库；团结工程 918MB → 394MB（compact 后 371.5MB），达到 < 400MB。
- 语言策略落地：`lua` 前端（自研 scanner）+ 调用/require 边；词法器新增 Lua 行注释、块注释、长字符串。
- MCP 工具补齐到 23 个：新增 `cg233_overview` / `cg233_symbol_path` / `cg233_asset_tree` / `cg233_asset_stats` / `cg233_asset_orphans` / `cg233_shader_info` / `cg233_config_tables` / `cg233_config_readers`。

**P2（预留）**
- UE：C++ 粗粒度图、`.uasset` 名称层、蓝图引用边。
- 团结资产桥：通过编辑器导出 guid→路径映射后启用 `guid→asset` 反查。
- 图算法增强：社区发现、模块热点、改动影响打分。

## 9. 命名与产物约定

| 项 | 值 |
| --- | --- |
| 仓库 / module / 二进制 | `codegraph233` |
| MCP server name | `codegraph233` |
| MCP 工具前缀 | `cg233_` |
| 项目配置 | `.codegraph233.json` |
| 索引目录 | `.codegraph233/`（gitignore） |
| 环境变量 | `CODEGRAPH233_ROOT` |
