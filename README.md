# codegraph233

**AST GraphRAG for game projects — Rust 实现。**

索引客户端 C# / Go / Rust / TypeScript / Lua、游戏资产与配表，通过 MCP 提供给 agent 查询。仓库已从 Go 完全迁移到 Rust（Go 代码已删除，历史基准见 [docs/benchmarks.md](docs/benchmarks.md)）。

## 能力（Rust 实现）

| 能力 | 状态 |
| --- | --- |
| C# 前端（namespace / 类型 / 成员 / 字段 / 属性 / enum / 方法 / 调用 / 构造 / 引用 / 继承 / 类型依赖） | ✅ |
| Go 前端（package / struct / interface / func / method / 跨包 receiver） | ✅ |
| Rust 前端（mod / struct / enum / trait / impl / fn / use 别名） | ✅ |
| TypeScript 前端（class / interface / enum / function / import 别名 / extends / implements） | ✅ |
| Lua 前端（模块函数 / `a.b` 与 `a:b` 方法 / local 函数与表 / require / 调用） | ✅ |
| prefab / scene（GameObject 层级、m_Script、嵌套 prefab、guid 引用） | ✅ |
| shader（Shader / property / include / multi_compile 关键字 / HLSL 函数） | ✅ |
| 媒体资产（图片宽高、音频采样率/码率/时长、视频时长、字体名与字形数、Spine atlas/skel） | ✅ |
| 配置表（xlsx / csv / tsv / json，表头与行数、读取点探测） | ✅ |
| 结构化资产（材质/动画/控制器/时间轴/混音器/SpriteAtlas/TMP 字体/asmdef）的 guid 引用 | ✅ |
| guid 映射（编辑器导出 TSV/CSV/JSON + 明文 `.meta` 补充 + 资产边物化） | ✅ |
| 增量索引（mtime/size）、prune、并行绑定 | ✅ |
| 增量重绑定（小 delta 只重算相关边，结果与全量重建一致） | ✅ |
| CLI：`init` / `index` / `status` / `compact` / `register` / `projects` / `install-mcp` | ✅ |
| MCP stdio + 23 个工具 | ✅ |
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
```

* 自动更新默认开启：`index` / `serve` 启动时在后台检查，24 小时最多一次；发现新版本会下载、
  替换二进制并提示重启 MCP 客户端。`--no-update` 可跳过本次检查。
* 设置文件：`~/.codegraph233/settings.json`（`auto_update`、`interval_hours`、`mirror`、`mirrors`、
  `last_check`、`last_version`）。可直接编辑。
* 环境变量：`CODEGRAPH233_MIRROR`（覆盖镜像）、`CODEGRAPH233_AUTO_UPDATE=0`（关闭自动更新）、
  `CODEGRAPH233_NO_UPDATE=1`（本次不检查）。
* 默认镜像顺序：`ghfast.top` → `gh-proxy.com` → `ghproxy.net` → GitHub 直连；任一成功即停止。
  `mirror: "none"` 表示只用直连。下载走系统 `curl`，不引入新的 HTTP 依赖。
* Windows 下正在运行的 exe 无法覆盖，更新采用“重命名旧文件 → 写入新文件”，因此更新后需重启
  MCP 客户端；遗留的 `.old-*` 与旧版本 exe 会在下次启动时自动清理。
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
* `languages` 省略默认 `["csharp"]`；`["auto"]` 表示全部已实现前端（csharp / go / rust / typescript / lua）。
* 索引产物：`.codegraph233/index.db`（gitignore，可重建）。schema 版本变化时 `index` 会自动重建，`status` 会提示 `stale schema`。
* `codegraph233 compact`：`ANALYZE` + `VACUUM`，回收增量索引产生的空闲页。

## MCP 工具（23）

代码图：`cg233_status` / `cg233_index` / `cg233_overview` / `cg233_projects` / `cg233_project_health` /
`cg233_find_symbol` / `cg233_get_symbol` / `cg233_symbol_path` / `cg233_file_outline` /
`cg233_find_referencing_symbols` / `cg233_call_graph` / `cg233_impact` / `cg233_search_pattern`

资产图：`cg233_asset_search` / `cg233_asset_info` / `cg233_asset_refs` / `cg233_asset_tree` /
`cg233_asset_stats` / `cg233_asset_orphans` / `cg233_guid_lookup`

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
