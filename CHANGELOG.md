# Changelog

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
