package assets

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path"
	"regexp"
	"strings"

	"github.com/neko233-com/codegraph233/internal/model"
)

var (
	reName     = regexp.MustCompile(`(?m)^[ \t]*m_Name:[ \t]*(\S[^\n]*?)[ \t]*$`)
	reGUID     = regexp.MustCompile(`guid:\s*([0-9a-fA-F]{32})`)
	reShader   = regexp.MustCompile(`m_Shader:\s*\{[^}]*guid:\s*([0-9a-fA-F]{32})`)
	reSourceFt = regexp.MustCompile(`m_SourceFontFile:\s*\{[^}]*guid:\s*([0-9a-fA-F]{32})`)
	reAtlasW   = regexp.MustCompile(`m_AtlasWidth:\s*(\d+)`)
	reAtlasH   = regexp.MustCompile(`m_AtlasHeight:\s*(\d+)`)
	reAtlasPop = regexp.MustCompile(`m_AtlasPopulationMode:\s*(\d+)`)
	reUnicode  = regexp.MustCompile(`m_Unicode:\s*\d+`)
	reGlyph    = regexp.MustCompile(`(?m)^\s*-\s*m_Index:`)
	reMetaGUID = regexp.MustCompile(`(?m)^guid:\s*([0-9a-fA-F]{32})\s*$`)
	reMetaFile = regexp.MustCompile(`(?m)^fileFormatVersion:`)
	reSkelAnim = regexp.MustCompile(`"animations"\s*:`)
	reSkelBone = regexp.MustCompile(`"bones"\s*:`)
)

// textAssetExts are YAML/JSON assets whose references matter more than bytes.
var textAssetExts = map[string]bool{
	".mat": true, ".asset": true, ".anim": true, ".controller": true,
	".mixer": true, ".playable": true, ".asmdef": true, ".spriteatlas": true,
	".preset": true, ".renderTexture": true, ".shadervariants": true,
}

// mediaExts are binary assets: only headers are inspected.
var mediaExts = map[string]bool{
	".png": true, ".jpg": true, ".jpeg": true, ".tga": true, ".psd": true,
	".exr": true, ".tif": true, ".tiff": true, ".bmp": true, ".gif": true,
	".webp": true, ".hdr": true, ".dds": true,
	".wav": true, ".mp3": true, ".ogg": true, ".oga": true, ".m4a": true,
	".aac": true, ".flac": true, ".aif": true, ".aiff": true, ".wma": true,
	".xm": true, ".it": true, ".mod": true,
	".mp4": true, ".mov": true, ".webm": true, ".mkv": true, ".avi": true, ".m4v": true,
	".ttf": true, ".otf": true, ".ttc": true,
	".skel": true,
}

// AssetKind classifies a path into the public asset taxonomy.
func AssetKind(relPath string) string {
	ext := strings.ToLower(path.Ext(relPath))
	switch ext {
	case ".png", ".jpg", ".jpeg", ".tga", ".psd", ".exr", ".tif", ".tiff",
		".bmp", ".gif", ".webp", ".hdr", ".dds":
		return "image"
	case ".wav", ".mp3", ".ogg", ".oga", ".m4a", ".aac", ".flac", ".aif",
		".aiff", ".wma", ".xm", ".it", ".mod":
		return "audio"
	case ".mp4", ".mov", ".webm", ".mkv", ".avi", ".m4v":
		return "video"
	case ".ttf", ".otf", ".ttc":
		return "font"
	case ".atlas":
		return "spine_atlas"
	case ".skel":
		return "spine_data"
	case ".mat":
		return "material"
	case ".anim":
		return "animation_clip"
	case ".controller":
		return "animator_controller"
	case ".mixer":
		return "audio_mixer"
	case ".playable":
		return "timeline"
	case ".asmdef":
		return "assembly_def"
	case ".spriteatlas":
		return "sprite_atlas"
	case ".meta":
		return "meta"
	case ".shader", ".hlsl", ".cginc", ".compute":
		return "shader"
	case ".asset":
		return "scriptable_asset"
	case ".json":
		return "json_asset"
	}
	return "asset"
}

// AssetExt reports whether a file should be indexed as an asset.
func AssetExt(relPath string) bool {
	ext := strings.ToLower(path.Ext(relPath))
	if mediaExts[ext] || textAssetExts[ext] || ext == ".atlas" || ext == ".meta" {
		return true
	}
	// spine JSON exports live next to .atlas files; keep them cheap and explicit
	if ext == ".json" && (strings.HasSuffix(relPath, ".spine.json") || strings.Contains(strings.ToLower(relPath), "spine")) {
		return true
	}
	return false
}

// ParseAsset dispatches one asset file.  Binary formats only read headers;
// text formats are capped by maxText bytes.
func ParseAsset(absPath, relPath, root string, maxText int64) *model.FileResult {
	kind := AssetKind(relPath)
	res := &model.FileResult{RelPath: relPath, Root: root, Lang: "asset"}
	key := "asset:" + root + ":" + relPath
	st, err := os.Stat(absPath)
	if err != nil {
		res.Errors = append(res.Errors, err.Error())
		return res
	}
	name := strings.TrimSuffix(path.Base(relPath), path.Ext(relPath))
	meta := map[string]string{"format": strings.TrimSuffix(path.Ext(relPath), "."), "bytes": fmt.Sprint(st.Size())}

	if kind == "meta" {
		return parseMeta(absPath, relPath, root, res)
	}

	if mediaExts[strings.ToLower(path.Ext(relPath))] {
		for k, v := range mediaMeta(absPath, relPath, st.Size()) {
			meta[k] = v
		}
		res.Assets = append(res.Assets, model.Asset{
			Key: key, Root: root, Kind: kind, Path: relPath, Name: name, Meta: meta,
		})
		return res
	}

	capBytes := maxText
	if kind == "scriptable_asset" {
		// TMP font atlases routinely exceed 30MB and their character table can
		// sit near the end of the file.
		capBytes = 48 << 20
	}
	data := readCapped(absPath, capBytes)
	switch kind {
	case "spine_atlas":
		pages := 0
		for _, line := range strings.Split(string(data), "\n") {
			l := strings.TrimSpace(line)
			if l == "" || strings.Contains(l, ":") {
				continue
			}
			low := strings.ToLower(l)
			if strings.HasSuffix(low, ".png") || strings.HasSuffix(low, ".jpg") || strings.HasSuffix(low, ".webp") {
				pages++
			}
		}
		meta["pages"] = fmt.Sprint(pages)
	case "json_asset":
		var doc map[string]any
		if json.Unmarshal(data, &doc) == nil {
			if sk, ok := doc["skeleton"].(map[string]any); ok {
				if v, ok := sk["spine"].(string); ok {
					meta["spine_version"] = v
				}
				meta["kind"] = "spine_json"
			}
			if b, ok := doc["bones"].([]any); ok {
				meta["bones"] = fmt.Sprint(len(b))
			}
			if a, ok := doc["animations"].(map[string]any); ok {
				meta["animations"] = fmt.Sprint(len(a))
			}
			if s, ok := doc["skins"].([]any); ok {
				meta["skins"] = fmt.Sprint(len(s))
			}
		}
	case "assembly_def":
		var doc struct {
			Name           string   `json:"name"`
			References     []string `json:"references"`
			IncludePlatfms []string `json:"includePlatforms"`
		}
		if json.Unmarshal(data, &doc) == nil {
			if doc.Name != "" {
				meta["assembly"] = doc.Name
				name = doc.Name
			}
			meta["references"] = strings.Join(doc.References, ",")
			meta["platforms"] = strings.Join(doc.IncludePlatfms, ",")
		}
	case "material":
		if m := reShader.FindSubmatch(data); m != nil {
			meta["shader_guid"] = string(m[1])
		}
	case "scriptable_asset":
		// TMP font assets and Spine SkeletonDataAsset are recognizable by fields
		if bytes.Contains(data, []byte("m_AtlasPopulationMode")) || bytes.Contains(data, []byte("m_CharacterTable")) {
			meta["kind"] = "tmp_font"
			if m := reAtlasW.FindSubmatch(data); m != nil {
				meta["atlas_width"] = string(m[1])
			}
			if m := reAtlasH.FindSubmatch(data); m != nil {
				meta["atlas_height"] = string(m[1])
			}
			if m := reAtlasPop.FindSubmatch(data); m != nil {
				meta["atlas_population_mode"] = string(m[1])
			}
			if m := reSourceFt.FindSubmatch(data); m != nil {
				meta["source_font_guid"] = string(m[1])
			}
			meta["characters"] = fmt.Sprint(len(reUnicode.FindAllSubmatch(data, -1)))
			meta["glyphs"] = fmt.Sprint(len(reGlyph.FindAllSubmatch(data, -1)))
		} else if bytes.Contains(data, []byte("skeletonJSON")) || bytes.Contains(data, []byte("atlasAssets")) {
			meta["kind"] = "spine_data"
			meta["animations"] = fmt.Sprint(bytes.Count(data, []byte("m_Animations")))
		}
	default:
		if bytes.Contains(data, []byte("m_AnimationClips")) {
			meta["kind"] = "animator_controller"
		}
	}
	if m := reName.FindSubmatch(data); m != nil {
		name = strings.Trim(string(m[1]), `"'`)
	}
	if meta["kind"] != "" && (kind == "asset" || kind == "scriptable_asset" || kind == "json_asset") {
		kind = meta["kind"]
	}

	res.Assets = append(res.Assets, model.Asset{
		Key: key, Root: root, Kind: kind, Path: relPath, Name: name, Meta: meta,
	})

	if kind == "material" && meta["shader_guid"] != "" {
		res.AssetEdges = append(res.AssetEdges, model.AssetEdge{
			Src: key, Dst: "guid:" + meta["shader_guid"], Kind: "uses_shader", Detail: "m_Shader", Root: root,
		})
	}
	for _, m := range reGUID.FindAllSubmatch(data, 400) {
		g := string(m[1])
		if g == meta["shader_guid"] {
			continue
		}
		res.AssetEdges = append(res.AssetEdges, model.AssetEdge{
			Src: key, Dst: "guid:" + g, Kind: "references", Detail: "guid", Root: root,
		})
	}
	return res
}

// parseMeta contributes a guid -> path row for plaintext (Unity) meta files.
func parseMeta(absPath, relPath, root string, res *model.FileResult) *model.FileResult {
	data := readCapped(absPath, 4096)
	if !reMetaFile.Match(data) {
		return res
	}
	m := reMetaGUID.FindSubmatch(data)
	if m == nil {
		return res
	}
	assetPath := strings.TrimSuffix(relPath, ".meta")
	// Path stays the .meta path: InsertAssets cleans by path, and using the
	// target asset path here would delete that asset's own nodes and edges.
	res.Assets = append(res.Assets, model.Asset{
		Key: "guid:" + string(m[1]), Root: root, Kind: "guid_ref", Path: relPath,
		Name: path.Base(assetPath), FileID: string(m[1]),
		Meta: map[string]string{
			"guid": string(m[1]), "linked": "1", "target_path": assetPath,
		},
	})
	return res
}

func readCapped(absPath string, max int64) []byte {
	st, err := os.Stat(absPath)
	if err != nil {
		return nil
	}
	if st.Size() < max {
		max = st.Size()
	}
	if max <= 0 {
		return nil
	}
	f, err := os.Open(absPath)
	if err != nil {
		return nil
	}
	defer f.Close()
	buf := make([]byte, max)
	n, _ := io.ReadFull(f, buf)
	return buf[:n]
}
