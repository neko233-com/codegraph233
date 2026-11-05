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
| Lua | planned | **未实现** | 仅预留策略位与配置项；当前版本遇到 `.lua` 不索引、不报错，`status` 中标注 planned |

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
| 配表 | 已实现 | xlsx（每个 sheet 一张表，**自动挑字段名行**）、csv/tsv、json；表头/行数/列数/说明行；读取点与热重载探测 |

## 4. MCP 契约

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

`cg233_status` / `cg233_index` / `cg233_overview` / `cg233_find_symbol` / `cg233_get_symbol` / `cg233_file_outline` / `cg233_find_referencing_symbols` / `cg233_call_graph` / `cg233_symbol_path` / `cg233_impact` / `cg233_search_pattern` / `cg233_asset_search` / `cg233_asset_tree` / `cg233_asset_refs` / `cg233_shader_info` / `cg233_config_tables` / `cg233_config_readers`

## 5. 项目接入与分发

- 项目配置文件：`.codegraph233.json`（提交到仓库）。
- 索引产物：`.codegraph233/`（必须 gitignore，可随时删除重建）。
- 多根目录：`roots[]`，`role` 只是约定（frontend / backend / shared / assets），后端可空缺。
- 一键安装：`install.ps1` / `install.sh` → `go install` 到 `$(go env GOPATH)/bin` 并补 PATH。
- MCP 注册：`codegraph233 install-mcp`，写入 codex / cursor / vscode / air / cowork / workbuddy / 项目根 `.mcp.json`；每条都带 `serve --root <项目>`，因此从后端仓库目录启动也能查同一份图。

## 6. 非目标（明确不做）

1. 不做编译级类型检查或语法诊断（不替代 IDE / Roslyn）。
2. 不做代码改写、重命名、批量重构（不做 Serena 的编辑类工具）。
3. 不做 UE 的 C++ / 蓝图 / 资产图（策略位保留）。
4. 不尝试解密团结引擎的 `.meta` guid（需编辑器桥；当前保留 guid 关系，不做 guid→路径）。
5. 不做远端服务、多用户协作、权限体系（本地 stdio 单机工具）。
6. 不为「改表现」或「提帧」类目的服务（那是性能工具的事）。

## 7. 验收标准

| 维度 | 标准 |
| --- | --- |
| 覆盖率 | 抽样 100 个 C# 类型/方法：声明召回 ≥ 95%；调用边召回 ≥ 80%（含 `name` 级） |
| 索引速度 | 2 万文件全量 ≤ 3 分钟；无改动增量 ≤ 40 秒 |
| 查询延迟 | 单次 `find_symbol` / `file_outline` < 200ms（30 万符号级索引） |
| 内存 | 全量重建峰值 ≤ 4GB；稳态查询 < 500MB |
| 稳定性 | 单文件解析失败不影响整批；索引中断后可增量续跑 |
| 可解释性 | 每条跨文件边必须带置信度；未解析引用可统计 |

## 8. 演进路线

**P0（已完成，2026-11-05）**
- C# 语义图、Go 语义图、prefab/scene、shader、配表、多根、引擎识别、SQLite 增量、17 个 MCP 工具、一键全局安装、7 客户端接入。

**P1（下一步）**
- 增量重绑定：只重算变更文件相关边（当前无改动也要 36s，因为全局 rebind）。
- 索引体积：裁剪低价值 `references`、按需压缩，目标 DB < 400MB。
- 语言策略落地：`lua` 前端（tree-sitter 或自研 scanner）+ 对应资产（lua 配置表、热更脚本）。

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
