// Package assets indexes non-code game data: Unity/Tuanjie prefabs and scenes
// (YAML), ShaderLab shaders and business config tables.
package assets

import (
	"bufio"
	"fmt"
	"os"
	"path"
	"regexp"
	"sort"
	"strings"

	"github.com/neko233-com/game-project-graph/internal/model"
)

var (
	docHeader   = regexp.MustCompile(`^--- !u!(\d+) &(-?\d+)`)
	guidRef     = regexp.MustCompile(`guid:\s*([0-9a-fA-F]{32})`)
	scriptRef   = regexp.MustCompile(`m_Script:\s*\{[^}]*guid:\s*([0-9a-fA-F]{32})`)
	sourcePrefb = regexp.MustCompile(`m_SourcePrefab:\s*\{[^}]*guid:\s*([0-9a-fA-F]{32})`)
	nameLine    = regexp.MustCompile(`^\s*m_Name:\s*(.+?)\s*$`)
	gameObjRef  = regexp.MustCompile(`m_GameObject:\s*\{fileID:\s*(-?\d+)`)
	fatherRef   = regexp.MustCompile(`m_Father:\s*\{fileID:\s*(-?\d+)`)
	childRef    = regexp.MustCompile(`^\s*-\s*\{fileID:\s*(-?\d+)\}`)
)

// unityDoc is one YAML document inside a prefab/scene.
type unityDoc struct {
	classID   string
	fileID    string
	name      string
	gameObj   string
	father    string
	children  []string
	scriptGID string
	srcPrefab string
	guids     []string
}

// ParsePrefab indexes a .prefab or .unity file as an asset graph fragment.
func ParsePrefab(absPath, relPath, root string) *model.FileResult {
	res := &model.FileResult{RelPath: relPath, Root: root, Lang: "prefab"}
	f, err := os.Open(absPath)
	if err != nil {
		res.Errors = append(res.Errors, err.Error())
		return res
	}
	defer f.Close()

	docs := map[string]*unityDoc{}
	var order []*unityDoc
	var cur *unityDoc

	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 0, 256*1024), 8*1024*1024)
	lineNo := 0
	inChildren := false
	childrenIndent := 0
	for sc.Scan() {
		lineNo++
		line := sc.Text()
		if m := docHeader.FindStringSubmatch(line); m != nil {
			cur = &unityDoc{classID: m[1], fileID: m[2]}
			docs[cur.fileID] = cur
			order = append(order, cur)
			inChildren = false
			continue
		}
		if cur == nil {
			continue
		}
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "m_Name:") {
			if m := nameLine.FindStringSubmatch(line); m != nil && cur.name == "" {
				cur.name = strings.Trim(m[1], `"'`)
			}
			continue
		}
		if strings.HasPrefix(trimmed, "m_Children:") {
			inChildren = true
			childrenIndent = len(line) - len(strings.TrimLeft(line, " \t"))
			continue
		}
		if inChildren {
			indent := len(line) - len(strings.TrimLeft(line, " \t"))
			if m := childRef.FindStringSubmatch(line); m != nil && indent > childrenIndent {
				cur.children = append(cur.children, m[1])
				continue
			}
			if indent <= childrenIndent && trimmed != "" {
				inChildren = false
			}
		}
		if m := scriptRef.FindStringSubmatch(line); m != nil {
			cur.scriptGID = m[1]
			continue
		}
		if m := sourcePrefb.FindStringSubmatch(line); m != nil {
			cur.srcPrefab = m[1]
			continue
		}
		if m := gameObjRef.FindStringSubmatch(line); m != nil {
			cur.gameObj = m[1]
			continue
		}
		if m := fatherRef.FindStringSubmatch(line); m != nil {
			cur.father = m[1]
			continue
		}
		if m := guidRef.FindStringSubmatch(line); m != nil {
			cur.guids = append(cur.guids, m[1])
		}
	}
	if err := sc.Err(); err != nil {
		res.Errors = append(res.Errors, err.Error())
	}

	fileKey := "prefab:" + root + ":" + relPath
	kind := "prefab"
	if strings.HasSuffix(relPath, ".unity") {
		kind = "scene"
	}
	res.Assets = append(res.Assets, model.Asset{
		Key: fileKey, Root: root, Kind: kind, Path: relPath,
		Name: strings.TrimSuffix(path.Base(relPath), path.Ext(relPath)),
		Meta: map[string]string{"file_id": "", "docs": fmt.Sprint(len(order))},
	})

	var objects []*unityDoc
	for _, d := range order {
		if d.classID == "1" {
			objects = append(objects, d)
		}
	}
	// name lookup for components that point at their GameObject
	objName := map[string]string{}
	for _, o := range objects {
		objName[o.fileID] = o.name
	}

	objKeys := map[string]string{}
	for _, o := range objects {
		key := fmt.Sprintf("%s#%s", fileKey, o.fileID)
		objKeys[o.fileID] = key
		objKind := "prefab_object"
		if kind == "scene" {
			objKind = "scene_object"
		}
		res.Assets = append(res.Assets, model.Asset{
			Key: key, Root: root, Kind: objKind, Path: relPath, Name: o.name, FileID: o.fileID,
			Meta: map[string]string{"class_id": o.classID},
		})
		res.AssetEdges = append(res.AssetEdges, model.AssetEdge{
			Src: fileKey, Dst: key, Kind: "contains", Detail: "gameobject", Root: root,
		})
	}

	// hierarchy edges: Transform m_Father -> parent GameObject
	for _, d := range order {
		if d.father == "" {
			continue
		}
		parentObj := ""
		if pd, ok := docs[d.father]; ok {
			if pd.classID == "1" {
				parentObj = pd.fileID
			} else if pd.gameObj != "" {
				parentObj = pd.gameObj
			}
		}
		childObj := d.gameObj
		if d.classID == "1" {
			childObj = d.fileID
		}
		if parentObj != "" && childObj != "" && parentObj != childObj {
			res.AssetEdges = append(res.AssetEdges, model.AssetEdge{
				Src: objKeys[parentObj], Dst: objKeys[childObj], Kind: "contains", Detail: "child", Root: root,
			})
		}
	}

	guidSeen := map[string]bool{}
	addGUID := func(gid, kindOfEdge, detail, owner string) {
		if gid == "" || guidSeen[owner+"|"+gid+"|"+kindOfEdge] {
			return
		}
		guidSeen[owner+"|"+gid+"|"+kindOfEdge] = true
		dst := "guid:" + gid
		res.Assets = append(res.Assets, model.Asset{
			Key: dst, Root: root, Kind: "guid_ref", Name: gid, FileID: gid,
			Meta: map[string]string{"guid": gid},
		})
		res.AssetEdges = append(res.AssetEdges, model.AssetEdge{
			Src: owner, Dst: dst, Kind: kindOfEdge, Detail: detail, Root: root,
		})
	}

	for _, d := range order {
		owner := fileKey
		if d.classID == "1" {
			if k, ok := objKeys[d.fileID]; ok {
				owner = k
			}
		} else if d.gameObj != "" {
			if k, ok := objKeys[d.gameObj]; ok {
				owner = k
			}
		}
		if d.scriptGID != "" {
			addGUID(d.scriptGID, "uses_script", "m_Script", owner)
		}
		if d.srcPrefab != "" {
			addGUID(d.srcPrefab, "instantiates", "m_SourcePrefab", owner)
		} else {
			for _, g := range d.guids {
				addGUID(g, "references", "guid", owner)
			}
		}
	}
	return res
}

// Shader parsing -------------------------------------------------------------

var (
	shaderName   = regexp.MustCompile(`(?m)^\s*Shader\s+"([^"]+)"`)
	shaderProp   = regexp.MustCompile(`(?m)^\s*(_?[A-Za-z][A-Za-z0-9_]*)\s*\(\s*"([^"]*)"`)
	shaderPass   = regexp.MustCompile(`(?m)^\s*Pass\s*(\{)?\s*$|\bName\s+"([^"]+)"`)
	shaderInc    = regexp.MustCompile(`(?m)^\s*#include\s+"([^"]+)"`)
	shaderFunc   = regexp.MustCompile(`(?m)^\s*(?:inline\s+)?(?:half|float|fixed|double|void|int|uint|bool|vector|real)[0-9]?(?:x[0-9]+)?\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(`)
	shaderPragma = regexp.MustCompile(`(?m)^\s*#pragma\s+(?:multi_compile|shader_feature)[^\n]*`)
	cgStart      = regexp.MustCompile(`(?m)^\s*(CGPROGRAM|HLSLPROGRAM|CGINCLUDE|HLSLINCLUDE)\b`)
	cgEnd        = regexp.MustCompile(`(?m)^\s*(ENDCG|ENDHLSL)\b`)
)

// ParseShader indexes a ShaderLab file, its properties, passes, includes,
// HLSL functions and compile keywords.
func ParseShader(absPath, relPath, root string) (*model.FileResult, error) {
	res := &model.FileResult{RelPath: relPath, Root: root, Lang: "shader"}
	data, err := os.ReadFile(absPath)
	if err != nil {
		res.Errors = append(res.Errors, err.Error())
		return res, err
	}
	text := string(data)
	name := strings.TrimSuffix(path.Base(relPath), path.Ext(relPath))
	if m := shaderName.FindStringSubmatch(text); m != nil {
		name = m[1]
	}
	fileKey := "shader:" + root + ":" + relPath
	res.Assets = append(res.Assets, model.Asset{
		Key: fileKey, Root: root, Kind: "shader", Path: relPath, Name: name,
		Meta: map[string]string{"lines": fmt.Sprint(strings.Count(text, "\n") + 1)},
	})

	// properties
	props := map[string]bool{}
	for _, m := range shaderProp.FindAllStringSubmatch(text, 400) {
		prop := m[1]
		if props[prop] {
			continue
		}
		props[prop] = true
		res.Assets = append(res.Assets, model.Asset{
			Key: fileKey + "#prop:" + prop, Root: root, Kind: "shader_property", Path: relPath,
			Name: prop, Meta: map[string]string{"label": m[2]},
		})
		res.AssetEdges = append(res.AssetEdges, model.AssetEdge{
			Src: fileKey, Dst: fileKey + "#prop:" + prop, Kind: "declares", Detail: "property", Root: root,
		})
	}
	// passes (Name "X" inside SubShader counts as a pass name)
	passIdx := 0
	for _, m := range shaderPass.FindAllStringSubmatch(text, 64) {
		pname := m[2]
		if pname == "" {
			passIdx++
			pname = fmt.Sprintf("pass_%d", passIdx)
		}
		key := fileKey + "#pass:" + pname
		res.Assets = append(res.Assets, model.Asset{
			Key: key, Root: root, Kind: "shader_pass", Path: relPath, Name: pname,
		})
		res.AssetEdges = append(res.AssetEdges, model.AssetEdge{
			Src: fileKey, Dst: key, Kind: "contains", Detail: "pass", Root: root,
		})
	}
	// includes
	seenInc := map[string]bool{}
	for _, m := range shaderInc.FindAllStringSubmatch(text, 200) {
		inc := m[1]
		if seenInc[inc] {
			continue
		}
		seenInc[inc] = true
		res.AssetEdges = append(res.AssetEdges, model.AssetEdge{
			Src: fileKey, Dst: "include:" + inc, Kind: "includes", Detail: inc, Root: root,
		})
	}
	// keywords: skip separators and lone underscores produced by `#pragma X _`
	var kws []string
	seenKw := map[string]bool{}
	for _, m := range shaderPragma.FindAllString(text, 200) {
		fields := strings.Fields(m)
		for _, f := range fields {
			if strings.HasPrefix(f, "#pragma") {
				continue
			}
			if !identLike(f) || f == "_" {
				continue
			}
			if !seenKw[f] {
				seenKw[f] = true
				kws = append(kws, f)
			}
		}
	}
	sort.Strings(kws)
	if len(kws) > 0 {
		res.Assets[0].Meta["keywords"] = strings.Join(kws, ",")
	}
	// HLSL functions inside program blocks
	for _, m := range shaderFunc.FindAllStringSubmatchIndex(text, 500) {
		fn := text[m[2]:m[3]]
		if fn == "" || fn == "main" {
			continue
		}
		line := 1 + strings.Count(text[:m[2]], "\n")
		res.Symbols = append(res.Symbols, model.Symbol{
			Root: root, Lang: "shader", Kind: model.KindShaderFunc, Name: fn,
			Qualname: name + "." + fn, Namespace: name, StartLine: line, EndLine: line,
			Signature: fn, ParamCount: -1, Module: relPath,
		})
	}
	return res, nil
}

// Config tables --------------------------------------------------------------

// IsConfigCandidate decides whether a data file is worth parsing as a config table.
func IsConfigCandidate(relPath string) bool {
	ext := strings.ToLower(path.Ext(relPath))
	switch ext {
	case ".xlsx", ".csv", ".tsv":
		return true
	case ".json":
		lower := strings.ToLower(relPath)
		return strings.Contains(lower, "config") || strings.Contains(lower, "business")
	}
	return false
}

// ConfigDirTables scans a directory for config sources (xlsx/csv/tsv/json).
func ConfigDirTables(dir string, limit int) []string {
	var out []string
	_ = filepathWalk(dir, func(p string) {
		if len(out) >= limit {
			return
		}
		if IsConfigCandidate(p) {
			out = append(out, p)
		}
	}, 6)
	sort.Strings(out)
	return out
}
