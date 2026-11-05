package mcpserver

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/neko233-com/codegraph233/internal/indexer"
	"github.com/neko233-com/codegraph233/internal/query"
)

type toolDef struct {
	Name        string         `json:"name"`
	Description string         `json:"description"`
	InputSchema map[string]any `json:"inputSchema"`
}

func obj(props map[string]any, required ...string) map[string]any {
	m := map[string]any{"type": "object", "properties": props}
	if len(required) > 0 {
		m["required"] = required
	}
	return m
}

func str(desc string) map[string]any  { return map[string]any{"type": "string", "description": desc} }
func num(desc string) map[string]any  { return map[string]any{"type": "integer", "description": desc} }
func boolean(desc string) map[string]any {
	return map[string]any{"type": "boolean", "description": desc}
}

func toolDefs() []toolDef {
	defs := []toolDef{
		{
			Name: "cg233_status",
			Description: "Show index status: detected engine strategy (unity/tuanjie/ue), configured roots, languages, and graph counts. Call this first.",
			InputSchema: obj(map[string]any{}),
		},
		{
			Name: "cg233_index",
			Description: "Build or incrementally update the project graph (C#/Go symbols + calls/refs/inherits, prefabs/scenes, shaders, config tables). Safe to call repeatedly; unchanged files are skipped.",
			InputSchema: obj(map[string]any{
				"force": boolean("Rebuild every file instead of using mtime/size fingerprints"),
				"jobs":  num("Parallel parse workers (default: CPU count)"),
			}),
		},
		{
			Name: "cg233_overview",
			Description: "Project-level overview: strategy/engine, roots, languages, symbol kind histogram, top namespaces, asset kinds, config table count and whether config hot reload exists.",
			InputSchema: obj(map[string]any{}),
		},
		{
			Name: "cg233_find_symbol",
			Description: "Search declarations by name or qualified-name substring. Returns ids usable by the other tools. Prefer this over grepping when locating types, methods, fields or shader functions.",
			InputSchema: obj(map[string]any{
				"query": str("Name or qualname substring, e.g. FishChessUnit or GetDataById"),
				"kind":  str("Optional kind filter: class,struct,interface,enum,method,function,property,field,shader_func,config_table"),
				"lang":  str("Optional language filter: csharp, go, shader"),
				"root":  str("Optional root key from cg233_status"),
				"path":  str("Optional file path substring"),
				"limit": num("Max results (default 30)"),
			}, "query"),
		},
		{
			Name: "cg233_get_symbol",
			Description: "Read one symbol by id, optionally with its numbered source body.",
			InputSchema: obj(map[string]any{
				"id":             num("Symbol id from cg233_find_symbol"),
				"include_source": boolean("Include numbered source body (default true)"),
				"max_lines":      num("Max source lines (default 120)"),
			}, "id"),
		},
		{
			Name: "cg233_file_outline",
			Description: "List declarations in a file (or a group of files matching a path substring) ordered by position.",
			InputSchema: obj(map[string]any{
				"path":  str("File path or path substring, e.g. FishChessUnit_Transform.cs"),
				"root":  str("Optional root key"),
				"limit": num("Max symbols (default 400)"),
			}, "path"),
		},
		{
			Name: "cg233_find_referencing_symbols",
			Description: "Who references/calls this symbol (incoming edges with confidence). Mirrors Serena's find_referencing_symbols.",
			InputSchema: obj(map[string]any{
				"id":    num("Symbol id"),
				"limit": num("Max results (default 100)"),
			}, "id"),
		},
		{
			Name: "cg233_call_graph",
			Description: "Traverse callers (direction=in) or callees (direction=out) up to N hops. Edge kinds include calls, constructs, references, inherits, implements, embeds.",
			InputSchema: obj(map[string]any{
				"id":        num("Symbol id"),
				"direction": str("in (callers) or out (callees), default in"),
				"depth":     num("Hops, 1-4, default 1"),
				"kinds":     str("Optional comma separated edge kinds, e.g. calls,constructs"),
				"limit":     num("Max results (default 100)"),
			}, "id"),
		},
		{
			Name: "cg233_symbol_path",
			Description: "Find the shortest directed call/construct path between two symbols.",
			InputSchema: obj(map[string]any{
				"from":      num("Start symbol id"),
				"to":        num("Target symbol id"),
				"max_depth": num("Max hops (default 8)"),
			}, "from", "to"),
		},
		{
			Name: "cg233_impact",
			Description: "Change-impact analysis: incoming references for a symbol or for the first symbol in a file, plus affected files. Use before refactors.",
			InputSchema: obj(map[string]any{
				"id":    num("Symbol id (preferred)"),
				"path":  str("File path substring when id is omitted"),
				"depth": num("Transitive depth, 1-4, default 2"),
			}),
		},
		{
			Name: "cg233_search_pattern",
			Description: "Regex search across indexed files with optional context lines. Use when the AST graph does not model a construct (strings, YAML, config text).",
			InputSchema: obj(map[string]any{
				"pattern":       str("Go RE2 regular expression"),
				"glob":          str("Optional path substring filter, e.g. FishChess or *.cs"),
				"root":          str("Optional root key"),
				"max_results":   num("Default 50"),
				"context_lines": num("Lines of context before/after, default 2"),
			}, "pattern"),
		},
		{
			Name: "cg233_asset_search",
			Description: "Search prefab/scene objects, shaders, shader passes/properties and config tables by name, path or kind.",
			InputSchema: obj(map[string]any{
				"name":  str("Name or asset key substring"),
				"kind":  str("prefab, scene, prefab_object, scene_object, shader, shader_pass, shader_property, config_table, config_source, guid_ref"),
				"root":  str("Optional root key"),
				"path":  str("Optional path substring"),
				"limit": num("Default 50"),
			}),
		},
		{
			Name: "cg233_asset_tree",
			Description: "Show a prefab/scene hierarchy (GameObjects and contains edges) for a path substring.",
			InputSchema: obj(map[string]any{
				"path":  str("Prefab/scene path substring, e.g. W2D_FishingBattle.prefab"),
				"root":  str("Optional root key"),
				"limit": num("Default 500"),
			}, "path"),
		},
		{
			Name: "cg233_asset_refs",
			Description: "List asset relationships (instantiates / uses_script / references / includes / declares) for an exact asset key, in or out.",
			InputSchema: obj(map[string]any{
				"key":       str("Exact asset key from cg233_asset_search"),
				"path":      str("Path substring when key is omitted"),
				"direction": str("out (default) or in"),
				"kinds":     str("Optional comma separated edge kinds: contains,references,instantiates,uses_script,includes,declares"),
				"limit":     num("Default 200"),
			}),
		},
		{
			Name: "cg233_asset_orphans",
			Description: "List resource assets nothing references (image/audio/video/font/spine/material/anim/timeline...), largest first. Use it to find cleanup candidates; entry points (prefab/scene/shader/config) are excluded by default.",
			InputSchema: obj(map[string]any{
				"root":  str("Optional root key"),
				"kinds": str("Optional comma separated asset kinds"),
				"limit": num("Default 200"),
			}),
		},
		{
			Name: "cg233_project_health",
			Description: "Index and asset health in one call: counts, unresolved refs, guid coverage, orphan/linked assets, DB size, top largest assets, active profile/features.",
			InputSchema: obj(map[string]any{
				"root": str("Optional root key"),
			}),
		},
		{
			Name: "cg233_guid_lookup",
			Description: "Resolve a guid (32 hex) or asset path substring to its asset path/type plus which prefabs/materials reference it and what it references. Needs a guid map exported from the editor (Tools/codegraph233/导出 GUID 映射); plaintext .meta is also auto-indexed.",
			InputSchema: obj(map[string]any{
				"guid":  str("32-hex guid or path substring"),
				"limit": num("Reference limit (default 100)"),
			}, "guid"),
		},
		{
			Name: "cg233_asset_stats",
			Description: "Asset inventory by family (image/audio/video/font/tmp_font/spine_data/material/animation/controller/timeline/config_table...): count, total bytes and the largest files. Use it to find heavy assets and audit what the project contains.",
			InputSchema: obj(map[string]any{
				"root":          str("Optional root key"),
				"top_per_kind":  num("Largest files per kind (default 3)"),
			}),
		},
		{
			Name: "cg233_asset_info",
			Description: "One asset by exact key or path substring: metadata (dimensions/duration/channels/font glyphs/TMP atlas/Spine animations/bytes) plus incoming and outgoing references.",
			InputSchema: obj(map[string]any{
				"key":  str("Exact asset key from cg233_asset_search"),
				"path": str("Path substring when key is omitted"),
				"limit": num("Edge limit (default 200)"),
			}),
		},
		{
			Name: "cg233_shader_info",
			Description: "ShaderLab summary: shader name, properties, passes, includes and multi_compile keywords.",
			InputSchema: obj(map[string]any{
				"path": str("Shader path substring"),
			}, "path"),
		},
		{
			Name: "cg233_config_tables",
			Description: "List detected business config tables (xlsx sheets / csv / tsv / json) with headers, row counts and source paths. Format is auto-detected, not hardcoded.",
			InputSchema: obj(map[string]any{
				"filter": str("Optional table/sheet name substring"),
				"root":   str("Optional root key"),
				"limit":  num("Default 200"),
			}),
		},
		{
			Name: "cg233_config_readers",
			Description: "List code locations that read configuration and report whether the project has config hot reload (detected by symbols such as ConfigHotReloadManager/ReloadConfig).",
			InputSchema: obj(map[string]any{
				"table": str("Optional table name for context"),
				"limit": num("Default 100"),
			}),
		},
	}
	for i := range defs {
		if defs[i].Name == "cg233_projects" {
			continue
		}
		if props, ok := defs[i].InputSchema["properties"].(map[string]any); ok {
			props["project"] = str("Optional project name/root (see cg233_projects); defaults to the current project")
		}
	}
	defs = append([]toolDef{{
		Name: "cg233_projects",
		Description: "List projects served by this MCP process (registered + current): name, root, whether an index exists, indexed time, file/symbol counts. Pass any name/root as the `project` argument of other tools.",
		InputSchema: obj(map[string]any{}),
	}}, defs...)
	return defs
}


func (s *Server) call(name string, args map[string]any) (string, error) {
	switch name {
	case "cg233_projects":
		return toJSON(s.projectRows())

	case "cg233_status":
		cfg, err := s.cfgFor(s.resolveRoot(argStr(args, "project", "")))
		if err != nil {
			return "", err
		}
		return toJSON(indexer.ReadStatus(cfg))

	case "cg233_index":
		if err := s.ensureIndex(args); err != nil {
			return "", err
		}
		cfg, err := s.cfgFor(s.resolveRoot(argStr(args, "project", "")))
		if err != nil {
			return "", err
		}
		return toJSON(indexer.ReadStatus(cfg))

	case "cg233_overview":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		o, err := eng.BuildOverview()
		if err != nil {
			return "", err
		}
		return toJSON(o)

	case "cg233_find_symbol":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		kinds := splitCSV(argStr(args, "kind", ""))
		hits, err := eng.SearchSymbols(
			argStr(args, "query", ""), kinds, argStr(args, "lang", ""),
			argStr(args, "root", ""), argStr(args, "path", ""), argInt(args, "limit", 30))
		if err != nil {
			return "", err
		}
		return toJSON(hits)

	case "cg233_get_symbol":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		h, err := eng.GetSymbol(int64(argInt(args, "id", 0)))
		if err != nil {
			return "", err
		}
		res := map[string]any{"symbol": h}
		if argBool(args, "include_source", true) {
			src, err := eng.Source(*h, argInt(args, "max_lines", 120))
			if err == nil {
				res["source"] = src
			}
		}
		return toJSON(res)

	case "cg233_file_outline":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		hits, err := eng.FileOutline(argStr(args, "path", ""), argStr(args, "root", ""), argInt(args, "limit", 400))
		if err != nil {
			return "", err
		}
		return toJSON(hits)

	case "cg233_find_referencing_symbols":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		hits, err := eng.RefsTo(int64(argInt(args, "id", 0)), argInt(args, "limit", 100))
		if err != nil {
			return "", err
		}
		return toJSON(hits)

	case "cg233_call_graph":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		dir := argStr(args, "direction", "in")
		hits, err := eng.Related(int64(argInt(args, "id", 0)), dir,
			splitCSV(argStr(args, "kinds", "")), argInt(args, "depth", 1), argInt(args, "limit", 100))
		if err != nil {
			return "", err
		}
		return toJSON(hits)

	case "cg233_symbol_path":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		steps, err := eng.CallPath(int64(argInt(args, "from", 0)), int64(argInt(args, "to", 0)), argInt(args, "max_depth", 8))
		if err != nil {
			return "", err
		}
		return toJSON(steps)

	case "cg233_impact":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		imp, err := eng.AnalyzeImpact(int64(argInt(args, "id", 0)), argStr(args, "path", ""), argInt(args, "depth", 2))
		if err != nil {
			return "", err
		}
		return toJSON(imp)

	case "cg233_search_pattern":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		hits, err := eng.SearchPattern(
			argStr(args, "pattern", ""), argStr(args, "glob", ""), argStr(args, "root", ""),
			argInt(args, "max_results", 50), argInt(args, "context_lines", 2))
		if err != nil {
			return "", err
		}
		return toJSON(hits)

	case "cg233_asset_search":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		hits, err := eng.SearchAssets(
			argStr(args, "name", ""), argStr(args, "kind", ""), argStr(args, "root", ""),
			argStr(args, "path", ""), argInt(args, "limit", 50))
		if err != nil {
			return "", err
		}
		return toJSON(hits)

	case "cg233_asset_tree":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		objs, edges, err := eng.PrefabTree(argStr(args, "path", ""), argStr(args, "root", ""), argInt(args, "limit", 500))
		if err != nil {
			return "", err
		}
		return toJSON(map[string]any{"objects": objs, "edges": edges})

	case "cg233_asset_refs":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		key := argStr(args, "key", "")
		if key == "" {
			pth := argStr(args, "path", "")
			if pth == "" {
				return "", fmt.Errorf("asset_refs needs key or path")
			}
			h, _, err := eng.AssetInfo("", pth, 1)
			if err != nil {
				return "", err
			}
			key = h.Key
		}
		hits, err := eng.AssetEdgesFiltered(key, argStr(args, "direction", "out"),
			splitCSV(argStr(args, "kinds", "")), argInt(args, "limit", 200))
		if err != nil {
			return "", err
		}
		return toJSON(hits)

	case "cg233_asset_orphans":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		hits, err := eng.OrphanAssets(argStr(args, "root", ""), splitCSV(argStr(args, "kinds", "")), argInt(args, "limit", 200))
		if err != nil {
			return "", err
		}
		return toJSON(hits)

	case "cg233_project_health":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		h, err := eng.HealthReport(argStr(args, "root", ""))
		if err != nil {
			return "", err
		}
		return toJSON(h)

	case "cg233_guid_lookup":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		hits, err := eng.LookupGuid(argStr(args, "guid", ""), argInt(args, "limit", 100))
		if err != nil {
			return "", err
		}
		return toJSON(hits)

	case "cg233_asset_stats":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		stats, err := eng.AssetStats(argStr(args, "root", ""), argInt(args, "top_per_kind", 3))
		if err != nil {
			return "", err
		}
		return toJSON(stats)

	case "cg233_asset_info":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		key, pth := argStr(args, "key", ""), argStr(args, "path", "")
		if key == "" && pth == "" {
			return "", fmt.Errorf("asset_info needs key or path")
		}
		h, edges, err := eng.AssetInfo(key, pth, argInt(args, "limit", 200))
		if err != nil {
			return "", err
		}
		return toJSON(map[string]any{"asset": h, "edges": edges})

	case "cg233_shader_info":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		found, err := eng.SearchAssets("", "shader", "", argStr(args, "path", ""), 5)
		if err != nil {
			return "", err
		}
		if len(found) == 0 {
			return "", fmt.Errorf("no shader matched %q", argStr(args, "path", ""))
		}
		shader := found[0]
		edges, _ := eng.AssetEdges(shader.Key, "out", 400)
		var props, passes, includes []string
		for _, e := range edges {
			switch e.Kind {
			case "declares":
				props = append(props, strings.TrimPrefix(e.Dst, shader.Key+"#prop:"))
			case "contains":
				if strings.Contains(e.Dst, "#pass:") {
					passes = append(passes, strings.TrimPrefix(e.Dst, shader.Key+"#pass:"))
				}
			case "includes":
				includes = append(includes, e.Detail)
			}
		}
		return toJSON(map[string]any{
			"shader": shader, "properties": props, "passes": passes,
			"includes": includes, "keywords": shader.Meta["keywords"],
		})

	case "cg233_config_tables":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		tables, err := eng.ConfigTables(argStr(args, "filter", ""), argStr(args, "root", ""), argInt(args, "limit", 200))
		if err != nil {
			return "", err
		}
		return toJSON(tables)

	case "cg233_config_readers":
		eng, err := s.engineFor(argStr(args, "project", ""))
		if err != nil {
			return "", err
		}
		readers, hot, err := eng.ConfigReaders(argStr(args, "table", ""), argInt(args, "limit", 100))
		if err != nil {
			return "", err
		}
		return toJSON(map[string]any{"hot_reload": hot, "readers": readers})
	}
	return "", fmt.Errorf("unknown tool: %s", name)
}

// query import is used by callers through types; keep the reference explicit.
var _ = query.Engine{}

func toJSON(v any) (string, error) {
	data, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return "", err
	}
	return string(data), nil
}

func argStr(m map[string]any, key, def string) string {
	if m == nil {
		return def
	}
	if v, ok := m[key].(string); ok && v != "" {
		return v
	}
	return def
}

func argInt(m map[string]any, key string, def int) int {
	if m == nil {
		return def
	}
	switch v := m[key].(type) {
	case float64:
		return int(v)
	case int:
		return v
	case int64:
		return int(v)
	case json.Number:
		if i, err := v.Int64(); err == nil {
			return int(i)
		}
	}
	return def
}

func argBool(m map[string]any, key string, def bool) bool {
	if m == nil {
		return def
	}
	if v, ok := m[key].(bool); ok {
		return v
	}
	return def
}

func splitCSV(s string) []string {
	if strings.TrimSpace(s) == "" {
		return nil
	}
	parts := strings.Split(s, ",")
	out := make([]string, 0, len(parts))
	for _, p := range parts {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	return out
}
