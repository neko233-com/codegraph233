# 基准与迁移记录（历史数据）

本文件保留 Go 实现（已删除）与 Rust 实现的实测数据，用于对比与回归参照。
所有数字来自同一台机器、同一仓库（47,960–48,101 文件，含 33,755 个 C# 文件）。

## 1. 解析（同一批 C# 文件，16 线程，排除规则一致）

| 实现 | 文件 | 字节 | 耗时 | 备注 |
| --- | --- | --- | --- | --- |
| Go（`bench-parse`） | 33,755 | 181 MB | **447 ms** | 符号更全（461k symbols / 2.06M refs） |
| Rust（`rust/src/csharp.rs`） | 33,755 | 181 MB | **168 ms** | 抽取更简（279k / 2.15M refs） |

## 2. 写库（100 万 refs + 80 万 edges，同 schema）

| 实现 | 存储引擎 | insert | DB |
| --- | --- | --- | --- |
| Go | `modernc.org/sqlite`（纯 Go） | **9,538 ms** | 99 MB |
| Rust | `rusqlite` bundled（原生 C SQLite） | **1,368 ms** | 99 MB |

差距 **7.0×**，根因是 SQLite 实现，不是语言。

## 3. 全量索引（Go 实现，含资产/guid/5 语言）

| 指标 | 数值 |
| --- | --- |
| 文件 | 48,101 |
| symbols / refs / edges | 271k / 1.74M / 795k |
| assets / asset_edges | 40,688 / 108,822 |
| 全量 `--force` | **65 s**（fast 档，`journal_mode=OFF`） |
| 无改动增量 | **12 s** |
| DB | 627 MB（compact 后） |
| 内存峰值 | 1.56 GB |
| GC | 141 次、CPU 0%、STW 49.6 ms、最大 1.83 ms、Go 堆 5–6 MB |

## 4. 其他历史结论

* 写库模式：`single`（整轮一个事务）64.9 s < `batch`（每 300 文件）69.4 s << `none`（无事务）155.1 s。**无事务更慢**。
* `--lite`（小 cache/mmap）实测未降低 RSS（2,443 MB），仅作显式档位。
* `journal_mode=OFF + synchronous=OFF + mmap` 是提速主因（去掉日志与 fsync）。
* 低优先级（Windows `BELOW_NORMAL`）+ `--cpus/--jobs` 是避免编辑器卡顿的实际手段。

## 5. Rust 迁移

* 目标：完全用 Rust 重写（索引 + 存储 + MCP），不再保留 Go。
* 现状（0.3.1）：C# / Go / Rust / TypeScript / Lua 前端、SQLite 存储、增量重绑定、prefab / shader / 结构化资产 / 配表 / guid、23 个 MCP 工具全部迁移完成。

## 6. Rust 全量索引（0.3.1，团结工程）

| 指标 | 数值 |
| --- | --- |
| 文件 | 49,599（C# 11,418 / Go 5,299 / 资产+meta 等） |
| symbols / refs / edges | 356,326 / 2,170,450 / 990,679 |
| assets / asset_edges | 40,648 / 100,807 |
| 全量 `index`（冷库，fast 档） | **19–22 s** |
| 无改动 `index` | **< 10 ms** |
| 单文件改动（增量重绑定） | **3.7 s** |
| DB（首个 v3 全量，未 compress） | **426 MB** |
| DB（`compact` 后） | **384 MB**（0.2.0 同库 918 MB） |

增量重绑定与 `--force` 全量重建逐边一致，由 `cargo test` 中的 `indexer::incremental_rebind_matches_full_rebuild` 固定。
