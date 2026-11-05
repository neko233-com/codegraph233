# codegraph233

**AST GraphRAG for game projects — Rust 实现。**

索引客户端 C#（后续阶段补齐 Go / Rust / TypeScript）、游戏资产与配表，通过 MCP 提供给 agent 查询。仓库已从 Go 完全迁移到 Rust（Go 代码已删除，历史基准见 [docs/benchmarks.md](docs/benchmarks.md)）。

## 当前状态（阶段 1 完成）

| 能力 | 状态 |
| --- | --- |
| C# 前端（namespace / 类型 / 成员 / 字段 / 属性 / enum / 方法 / 调用 / 构造 / 引用 / 继承 / 类型依赖） | ✅ |
| SQLite 存储（与历史 Go 版同 schema，可复用已有索引） | ✅ |
| 增量索引（mtime/size）、删除清理、引用绑定（并行） | ✅ |
| CLI：`init` / `index` / `status` / `serve` / `install-mcp` | ✅ |
| MCP stdio + 14 个工具（符号/引用/调用图/影响面/正则检索/资产/guid/健康） | ✅ |
| Go / Rust / TypeScript 前端 | ⏳ 阶段 2 |
| prefab / scene / shader / 资产（图片/音频/视频/字体/Spine）/ 配表 / guid 映射 | ⏳ 阶段 2 |
| 多项目注册表（`project` 参数） | ⏳ 阶段 2 |

> 未实现的能力在 MCP 里返回空数组或明确提示，不会静默给错数据。

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

依赖 Rust 1.80+（开发机为 1.99）。`rusqlite` 使用 bundled 原生 SQLite，首次编译约 1–2 分钟。

## 快速开始

```sh
cd <game-project>
codegraph233 init        # 生成 .codegraph233.json（自动识别团结/Unity）
codegraph233 index       # 全量索引，之后自动增量
codegraph233 status      # 统计
codegraph233 serve       # MCP stdio（客户端会自动拉起）
```

## 性能（Rust）

本仓实测（11,418 个 C# 文件，`--profile code`）：

| 指标 | 数值 |
| --- | --- |
| 全量索引 | **16 s**（Go 版同任务约 40–50 s，历史数据见 benchmarks） |
| 解析（33,755 文件） | 168 ms（Go 447 ms） |
| 写库（100 万 refs + 80 万 edges） | 1.37 s（Go 9.54 s，**7×**） |
| 无 GC | ✅（Rust 无 GC，Go 版 GC 本身也只占 0% CPU） |

RSS 仍由 SQLite 页缓存决定（~2.4GB 量级），换语言不会改变这一点。

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
* `languages` 省略默认 `["csharp"]`；`["auto"]` 表示全部已实现前端（阶段 2 生效）。
* 索引产物：`.codegraph233/index.db`（gitignore，可重建）。

## MCP 工具

`cg233_status` / `cg233_index` / `cg233_find_symbol` / `cg233_get_symbol` / `cg233_file_outline` / `cg233_find_referencing_symbols` / `cg233_call_graph` / `cg233_impact` / `cg233_search_pattern` / `cg233_asset_search` / `cg233_asset_info` / `cg233_asset_refs` / `cg233_guid_lookup` / `cg233_project_health`

## 开发

```sh
cargo build --release
cargo test
```

MIT.
