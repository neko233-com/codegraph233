// codegraph233 — GUID 映射导出器（Unity / 团结引擎 Editor）
//
// 用途：codegraph233 需要 guid → 资产路径 的映射，才能把 prefab / 材质里的
// `guid: xxxx` 解析成真实资产。团结引擎的 .meta GUID 大部分是加密的，
// 只有编辑器里的 AssetDatabase.AssetPathToGUID 能给出与 YAML 一致的 32 位值。
//
// 安装：把本文件复制到项目的任意 Editor 目录（例如 Assets/Editor/）。
// 菜单：Tools/codegraph233/导出 GUID 映射
// 批处理：
//   Tuanjie.exe -batchmode -quit -projectPath <项目> \
//     -executeMethod Codegraph233.Editor.Codegraph233GuidExporter.ExportBatch
//
// 产物：Temp/codegraph233-guid-map.tsv（guid \t path \t type）
// codegraph233 索引时会自动发现 Temp/codegraph233-guid-map.tsv。

#if UNITY_EDITOR
using System.IO;
using System.Text;
using UnityEditor;
using UnityEngine;

namespace Codegraph233.Editor
{
    public static class Codegraph233GuidExporter
    {
        private const string DefaultOutput = "Temp/codegraph233-guid-map.tsv";

        [MenuItem("Tools/codegraph233/导出 GUID 映射")]
        public static void ExportMenu()
        {
            Export(DefaultOutput, true);
        }

        /// <summary>Batchmode entry point (-executeMethod).</summary>
        public static void ExportBatch()
        {
            Export(DefaultOutput, true);
        }

        public static void Export(string outPath, bool log)
        {
            var paths = AssetDatabase.GetAllAssetPaths();
            var sb = new StringBuilder(paths.Length * 80);
            sb.Append("guid\tpath\ttype\n");

            int count = 0;
            foreach (var p in paths)
            {
                if (!p.StartsWith("Assets/") && !p.StartsWith("Packages/"))
                {
                    continue;
                }

                var guid = AssetDatabase.AssetPathToGUID(p);
                if (string.IsNullOrEmpty(guid))
                {
                    continue;
                }

                var type = AssetDatabase.GetMainAssetTypeAtPath(p);
                sb.Append(guid).Append('\t').Append(p).Append('\t')
                    .Append(type != null ? type.Name : string.Empty).Append('\n');
                count++;
            }

            var full = Path.GetFullPath(outPath);
            var dir = Path.GetDirectoryName(full);
            if (!string.IsNullOrEmpty(dir))
            {
                Directory.CreateDirectory(dir);
            }
            File.WriteAllText(full, sb.ToString(), new UTF8Encoding(false));

            if (log)
            {
                Debug.Log($"[codegraph233] guid map exported: {count} assets -> {full}");
            }
            if (Application.isBatchMode)
            {
                EditorApplication.Exit(0);
            }
        }
    }
}
#endif
