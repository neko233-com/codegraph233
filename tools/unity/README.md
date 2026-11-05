# GUID 映射导出（Unity / 团结引擎）

codegraph233 需要 `guid → 资产路径` 的映射，才能把 prefab / 材质 / 场景 YAML 里的
`guid: xxxx` 解析成真实资产（并给出反向的“谁引用了它”）。

* **Unity**：`.meta` 里是明文 32 位 guid，codegraph233 会自动扫描，无需导出。
* **团结引擎**：`.meta` GUID 大部分是加密 base64（本仓实测 41.8% 明文、58.2% 加密），
  prefab 里引用的资产有 86% 无法仅靠 `.meta` 反解。**必须用编辑器导出**。

## 一、交互式导出（推荐）

1. 把 [`Codegraph233GuidExporter.cs`](Codegraph233GuidExporter.cs) 复制到项目的任意 `Editor` 目录，例如 `Assets/Editor/`。
2. 菜单 `Tools/codegraph233/导出 GUID 映射`。
3. 产物：`Temp/codegraph233-guid-map.tsv`（`guid \t path \t type`，本仓 27,412 行 / 3.2MB / 约 2 秒）。

`codegraph233` 索引时会自动发现 `Temp/codegraph233-guid-map.tsv`，无需配置。

## 二、批处理导出（CI / 无人工）

```powershell
# 编辑器必须关闭
& "<Tuanjie目录>\Editor\Tuanjie.exe" -batchmode -quit `
    -projectPath "<项目路径>" `
    -executeMethod Codegraph233.Editor.Codegraph233GuidExporter.ExportBatch
```

若使用 Unity：

```bash
Unity -batchmode -quit -projectPath "$PWD" \
  -executeMethod Codegraph233.Editor.Codegraph233GuidExporter.ExportBatch
```

## 三、索引与验证

```sh
codegraph233 index            # 自动读取 Temp/codegraph233-guid-map.tsv
codegraph233 status           # guid_map_entries / guid_refs_mapped / guid_refs
```

MCP 侧：

* `cg233_guid_lookup(guid | path)` — 解析 guid 或路径，返回资产类型 + 被谁引用 + 引用了谁；
* `cg233_asset_refs(path=..., direction=in|out)` — 按路径查引用关系；
* `cg233_asset_info(path=...)` — 单资产元信息 + 双向边。

## 四、自定义位置

项目 `.codegraph233.json`：

```json
{
  "guid_map": ["Temp/codegraph233-guid-map.tsv", "build/guid-map.json"]
}
```

支持 `tsv` / `csv` / `json`，多个文件按顺序合并（后者覆盖前者）；只接受 32 位十六进制 guid。

## 五、资产变更后

资产新增/删除/改名后重新导出一次即可；映射表是全量替换，不会残留旧条目。
建议把它挂到打包流程的第一步（或在 CI 里紧挨 `codegraph233 index` 之前执行）。
