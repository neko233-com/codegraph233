# codegraph233

**Serena 的符号级工具 + Graphify 的图检索 + 对游戏工程的深度支持。**

一个 MCP server，把「客户端 C# + 服务端 Go + prefab/shader/业务配表」索引成一张可查询的图。通用代码图工具看不懂游戏项目里占工作量一半的资产与配表，codegraph233 补上这一块。

规范（目的、策略、非目标、验收）见 **[docs/SPEC.md](docs/SPEC.md)**。

## 能力

| 能力域 | 内容 |
| --- | --- |
| 代码语义图 | 跨文件类型/成员/调用/构造/引用/继承，边带 `exact` / `type` / `name` 置信度；Go 后端跨包解析与结构体嵌入 |
| 游戏数据图 | prefab/scene 的 GameObject 层级、`m_Script`、嵌套 prefab、资源 guid；ShaderLab 的 property/pass/include/keyword；配表的表头/行数/读取点/热重载 |
| 工程识别 | 自动判断团结（`t` 补丁号）/ Unity / UE（仅识别），多根目录（前端 + 后端，后端可空缺） |

## 资产谱系

| 家族 | 扩展名 | 元信息 |
| --- | --- | --- |
| image | png / jpg / tga / psd / exr / tif / bmp / gif / webp / hdr / dds | 宽高（PNG/JPEG/GIF/BMP/PSD/WEBP/TGA 头部解析） |
| audio | wav / mp3 / ogg / flac / m4a / aac / aif / wma / xm / it / mod | 采样率、声道、时长（WAV/MP3/OGG/FLAC/MP4 容器） |
| video | mp4 / mov / webm / mkv / avi / m4v | 时长、分辨率（mvhd/tkhd，支持 moov 在文件尾） |
| font | ttf / otf / ttc | 字体名、字形数（name/maxp 表） |
| tmp_font | `.asset` 中的 TMP_FontAsset | 图集宽高、字符数、字形数、源字体 guid |
| spine | `.skel` / `.atlas` / spine json | atlas 页数、骨骼数、动画数、spine 版本 |
| material | `.mat` | shader guid、贴图/属性引用 |
| animation | `.anim` / `.controller` / `.playable` / `.mixer` | 名称与内部引用（clip / state / track） |
| assembly | `.asmdef` | 程序集名、引用、平台 |
| shader | `.shader` / `.hlsl` / `.cginc` / `.compute` | property / pass / include / keyword / HLSL 函数 |
| config | xlsx / csv / tsv / json | 表头、行数、读取点、热重载 |

* **二进制资产只读头部**：图片/音频/视频约 8KB，字体按表偏移读 name/maxp，MP4 支持 moov 在文件尾（读尾部 256KB），从不加载整文件。
### GUID 关联（prefab ↔ 资产的真实关系）

* **编辑器导出（推荐，100% 覆盖）**：`tools/unity/Codegraph233GuidExporter.cs` 提供菜单 `Tools/codegraph233/导出 GUID 映射`，产物 `Temp/codegraph233-guid-map.tsv`（本仓 27,412 条 / 2 秒）。`codegraph233 index` 自动发现，产物全量替换（`guid_map` 表）。
* **明文 `.meta` 兜底**：索引仍会扫描 `**/*.meta`，接受 32 位明文 guid（本仓 41.8% 明文），只补充、不覆盖编辑器导出。
* **效果（本仓实测）**：4,933 个 guid 引用中 **4,771 个解析到资产（96.7%）**；`cg233_guid_lookup` 能回答“这个 guid 是谁、被谁引用、它引用了谁”，`cg233_asset_refs(path=...)` 能列出 prefab → 嵌套 prefab / 脚本 / 贴图 / 材质，边带 `dst_path` 与 `dst_type`。
* 团结加密 guid 仅靠 `.meta` 无法反解（无导出时覆盖率约 13.6%），`status` 会用 `guid_refs_mapped / guid_refs` 明确显示当前覆盖率。
* 资产新增/改名后重新导出一次即可；CI 可用 `-executeMethod Codegraph233.Editor.Codegraph233GuidExporter.ExportBatch`。

新增 MCP 工具：`cg233_asset_stats`（按家族统计数量/体积 + 最大文件）、`cg233_asset_info`（元信息 + 双向引用）、`cg233_guid_lookup`（guid/路径 → 资产 + 双向关系）。

## 缓存与排除

默认排除生成/缓存目录：`Library`、`Temp`、`Logs`、`obj`、`Build(s)`、`UserSettings`、`MemoryCaptures`、`Recordings`、`Bundles`、`StreamingAssets`、`LocalPackageCache` 等，以及 `.codely.packages`、`.codely`、`backup`、`_backup`、`备份`、`Documentation~`、`Samples~`、`Tests~`。
项目可在 `exclude` 追加（本仓追加了美术仓库 `Team-Art-Resources-for-project-sf` 与工具目录 `_Tools` / `_tools`）。

## 语言与引擎策略

| 语言 | 级别 | 状态 |
| --- | --- | --- |
| **C#** | 默认（default） | 完整支持：符号/调用/引用/继承 + 资产联动 |
| Go | 可选（optional） | 已实现：跨包解析、receiver/字段推断、结构体嵌入；需在 `languages` 显式开启 |
| Rust | 可选（optional） | 已实现：mod/struct/enum/trait/impl/fn、`use` 别名、调用/构造/实现边 |
| TypeScript / JavaScript | 可选（optional） | 已实现：class/interface/type/enum/function、import 别名、`new`/调用/继承 |
| Lua | 预留（planned） | **未实现**，当前不索引 |

| 引擎 | 状态 |
| --- | --- |
| 团结引擎 / Unity | 已实现（自动识别，资产层分别处理明文/加密 guid） |
| UE | 仅识别项目，图未实现 |

## 安装（一键全局）

Windows：

```powershell
./install.ps1
```

Linux / macOS / WSL：

```sh
./install.sh
```

脚本会检查 Go（1.24+，推荐 1.27）、`go install` 到 `$(go env GOPATH)/bin`、补 PATH，并注册 MCP。

注册范围用 `-Scope` / `--scope` 选择：

| scope | 写到哪里 | 条目形态 |
| --- | --- | --- |
| `project`（默认） | 项目内 `.codex` / `.cursor` / `.vscode` / `.air` / `.codely-cli` / `.workbuddy` / `.mcp.json` | `serve --root <项目绝对路径>`，锁定本项目 |
| `global` | 用户目录 `~/.codex` / `~/.cursor` / `~/.air` / `~/.codely-cli` / `~/.workbuddy`、VS Code 用户目录 | `serve`（不带 root），按客户端工作目录自动发现 `.codegraph233.json` |
| `both` | 两处都写 | 全局兜底 + 项目精确 |

```powershell
./install.ps1 -Scope both
./install.sh --scope both
codegraph233 install-mcp --scope both --print   # 只看不改
```

手动：`go install github.com/neko233-com/codegraph233/cmd/codegraph233@main`

## 快速开始

```sh
cd <game-project>
codegraph233 init          # 写 .codegraph233.json（自动识别引擎/语言/后端目录）
# 团结项目：先导出 guid 映射（见 tools/unity/README.md），再索引
codegraph233 index         # 全量索引，之后增量
codegraph233 status        # 识别结果 + 策略矩阵 + 统计
codegraph233 install-mcp   # 写各客户端 MCP 配置
```

`init` 会把 `/.codegraph233/` 追加进 `.gitignore`；索引是本地状态，不入库。

## 配置

`.codegraph233.json`（提交）：

```json
{
  "version": 1,
  "strategy": "auto",
  "roots": [
    { "key": "client", "path": ".", "role": "frontend" },
    { "key": "server", "path": "../server-project-sf-go", "role": "backend" }
  ],
  "languages": ["csharp", "go", "rust", "typescript"],
  "config_dirs": ["Team-Resources/BusinessConfig"],
  "exclude": ["Assets/Plugins", "_server"]
}
```

* `languages` 省略时默认为 `["csharp"]`；Go / Rust / TypeScript 需显式加入（`init` 会把检测到的语言写进配置）。
* `strategy`：`auto` | `unity` | `tuanjie` | `ue` | `generic`。
* `roots` 数量不限，缺失目录只标记 `missing`，不中断。
* 索引位置：`<project>/.codegraph233/index.db`（SQLite WAL）。

## MCP 工具

| 工具 | 用途 |
| --- | --- |
| `cg233_status` | 识别结果、策略矩阵、索引统计 |
| `cg233_index` | 建立/增量更新（`force`、`jobs`） |
| `cg233_overview` | 项目概览：策略、类型直方图、顶层命名空间、资产种类、配表与热重载 |
| `cg233_find_symbol` | 按名字/限定名查类型、方法、字段、shader 函数 |
| `cg233_get_symbol` | 符号详情 + 带行号源码 |
| `cg233_file_outline` | 文件结构 |
| `cg233_find_referencing_symbols` | 谁引用/调用了它（对标 Serena） |
| `cg233_call_graph` | 调用者/被调用者，多跳 |
| `cg233_symbol_path` | 两符号间最短调用路径 |
| `cg233_impact` | 改动影响面（类型会聚合其成员） |
| `cg233_search_pattern` | 正则检索 + 上下文（补图未覆盖的语法） |
| `cg233_asset_search` | prefab/scene 对象、shader、配表 |
| `cg233_asset_tree` | prefab/scene 层级 |
| `cg233_asset_refs` | instantiates / uses_script / references / includes（支持 `path` 与 `kinds` 过滤，边带 `dst_path`/`dst_type`） |
| `cg233_shader_info` | properties、pass、include、keyword |
| `cg233_guid_lookup` | guid / 路径 → 资产路径、类型、被谁引用、引用了谁 |
| `cg233_config_tables` | 配表清单（表头/行数/来源） |
| `cg233_config_readers` | 读取点 + 是否支持热重载 |

## 性能（实测）

客户端 + 后端一个索引（`Assets/**` + Go 服务）：

| | |
| --- | --- |
| 文件 | 48,101（代码 / prefab / shader / 配表 / 资产 / 20k `.meta`） |
| 符号 / 边 | 271k / 702k |
| 资产 | 40,688（image 6,895 · material 672 · spine 200+199 · audio 321 · font 3 · tmp_font 2 · timeline 120 · asmdef 159 · guid_ref 27k …） |
| GUID 映射 | 27,720 条；引用覆盖率 4,771 / 4,933 = **96.7%** |
| 全量 `--force` | ~108s |
| 无改动增量 | ~40s（引用重绑定仍是全局） |
| DB | ~1.0GB（WAL 在退出时 TRUNCATE 收敛） |
| 排除后减少 | 8,580 个美术备份/工具文件 |

解析按 CPU 并行，写库分批，峰值内存约 3GB（23k 文件全量重建）。

## 已知限制

* C# 前端是扫描器而非编译器：跨文件解析覆盖局部变量、参数、字段与明显的静态 receiver；剩余引用以 `name` / `name-multi` 标注。
* 团结引擎 `.meta` 的 guid 是加密的，prefab guid 无法反查资产路径（保留 guid 关系）。
* `--force` 是图过旧时的兜底；正常增量依赖文件 mtime/size。
* 无改动也要 36s 的原因是全局 rebind，P1 会改成按文件增量。

## 开发

```sh
go test ./...
go vet ./...
go build ./cmd/codegraph233
```

MIT.
