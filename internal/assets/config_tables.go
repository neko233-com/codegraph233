package assets

import (
	"encoding/csv"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path"
	"path/filepath"
	"strings"

	"github.com/xuri/excelize/v2"

	"github.com/neko233-com/game-project-graph/internal/model"
)

const maxConfigBytes = 16 << 20

// ParseConfigFile indexes one config source.  Format is auto-detected from the
// extension; xlsx sheets become separate tables.
func ParseConfigFile(absPath, relPath, root string) *model.FileResult {
	res := &model.FileResult{RelPath: relPath, Root: root, Lang: "config"}
	ext := strings.ToLower(path.Ext(relPath))
	switch ext {
	case ".xlsx":
		parseXlsx(res, absPath, relPath, root)
	case ".csv":
		parseDelimited(res, absPath, relPath, root, ',')
	case ".tsv":
		parseDelimited(res, absPath, relPath, root, '\t')
	case ".json":
		parseJSONConfig(res, absPath, relPath, root)
	}
	return res
}

func isSkippableSheet(name string) bool {
	lower := strings.ToLower(name)
	for _, bad := range []string{"说明", "readme", "cover", "sheet1"} {
		if strings.Contains(lower, bad) {
			return true
		}
	}
	return false
}

func parseXlsx(res *model.FileResult, absPath, relPath, root string) {
	st, err := os.Stat(absPath)
	if err != nil || st.Size() > maxConfigBytes {
		res.Errors = append(res.Errors, "xlsx too large or unreadable")
		return
	}
	f, err := excelize.OpenFile(absPath)
	if err != nil {
		res.Errors = append(res.Errors, err.Error())
		return
	}
	defer f.Close()
	fileKey := "config_file:" + root + ":" + relPath
	res.Assets = append(res.Assets, model.Asset{
		Key: fileKey, Root: root, Kind: "config_source", Path: relPath,
		Name: strings.TrimSuffix(path.Base(relPath), path.Ext(relPath)),
		Meta: map[string]string{"format": "xlsx"},
	})
	for _, sheet := range f.GetSheetList() {
		if isSkippableSheet(sheet) {
			continue
		}
		iter, err := f.Rows(sheet)
		if err != nil {
			continue
		}
		var preview [][]string
		count := 0
		for iter.Next() {
			row, _ := iter.Columns()
			if len(row) > 512 {
				row = row[:512]
			}
			if count < 6 {
				preview = append(preview, row)
			}
			count++
			if count > 200000 {
				break
			}
		}
		iter.Close()
		if count == 0 || len(preview) == 0 {
			continue
		}
		// Game sheets usually start with a Chinese description row, then the
		// field-name row, then types, then data. Pick the most identifier-like
		// row as headers instead of assuming row 0.
		headerRow := pickHeaderRow(preview)
		headers := cleanHeaders(preview[headerRow])
		desc := ""
		if headerRow > 0 {
			desc = strings.Join(cleanHeaders(preview[0]), " | ")
		}
		key := fileKey + "#" + sheet
		res.Assets = append(res.Assets, model.Asset{
			Key: key, Root: root, Kind: "config_table", Path: relPath, Name: sheet,
			Meta: map[string]string{
				"format":     "xlsx",
				"sheet":      sheet,
				"headers":    strings.Join(headers, ","),
				"columns":    fmt.Sprint(len(headers)),
				"rows":       fmt.Sprint(count - headerRow - 1),
				"header_row": fmt.Sprint(headerRow + 1),
				"description": desc,
			},
		})
		res.AssetEdges = append(res.AssetEdges, model.AssetEdge{
			Src: fileKey, Dst: key, Kind: "contains", Detail: "sheet", Root: root,
		})
	}
}

func identLike(s string) bool {
	if s == "" {
		return false
	}
	c := s[0]
	if !(c == '_' || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')) {
		return false
	}
	for i := 0; i < len(s); i++ {
		c := s[i]
		if !(c == '_' || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9')) {
			return false
		}
	}
	return true
}

// pickHeaderRow returns the index of the row whose cells look most like field
// identifiers; ties prefer the earliest row.
func pickHeaderRow(rows [][]string) int {
	best, bestScore := 0, -1.0
	for i, row := range rows {
		total, ok := 0, 0
		for _, c := range row {
			c = strings.TrimSpace(c)
			if c == "" {
				continue
			}
			total++
			if identLike(c) {
				ok++
			}
		}
		if total == 0 {
			continue
		}
		score := float64(ok) / float64(total)
		if score > bestScore {
			best, bestScore = i, score
		}
	}
	return best
}

func cleanHeaders(row []string) []string {
	out := make([]string, 0, len(row))
	for _, h := range row {
		h = strings.TrimSpace(strings.ReplaceAll(h, "\n", ""))
		if h == "" {
			continue
		}
		out = append(out, h)
	}
	return out
}

func parseDelimited(res *model.FileResult, absPath, relPath, root string, sep rune) {
	st, err := os.Stat(absPath)
	if err != nil || st.Size() > maxConfigBytes {
		res.Errors = append(res.Errors, "config file too large or unreadable")
		return
	}
	f, err := os.Open(absPath)
	if err != nil {
		res.Errors = append(res.Errors, err.Error())
		return
	}
	defer f.Close()
	r := csv.NewReader(f)
	r.Comma = sep
	r.FieldsPerRecord = -1
	r.LazyQuotes = true
	header, err := r.Read()
	if err != nil && err != io.EOF {
		res.Errors = append(res.Errors, err.Error())
		return
	}
	rows := 0
	for {
		_, err := r.Read()
		if err == io.EOF {
			break
		}
		if err != nil {
			break
		}
		rows++
		if rows > 500000 {
			break
		}
	}
	headers := cleanHeaders(header)
	key := "config_file:" + root + ":" + relPath
	res.Assets = append(res.Assets, model.Asset{
		Key: key, Root: root, Kind: "config_table", Path: relPath,
		Name: strings.TrimSuffix(path.Base(relPath), path.Ext(relPath)),
		Meta: map[string]string{
			"format":  strings.TrimPrefix(path.Ext(relPath), "."),
			"headers": strings.Join(headers, ","),
			"columns": fmt.Sprint(len(headers)),
			"rows":    fmt.Sprint(rows),
		},
	})
}

func parseJSONConfig(res *model.FileResult, absPath, relPath, root string) {
	st, err := os.Stat(absPath)
	if err != nil || st.Size() > maxConfigBytes {
		res.Errors = append(res.Errors, "json too large or unreadable")
		return
	}
	data, err := os.ReadFile(absPath)
	if err != nil {
		res.Errors = append(res.Errors, err.Error())
		return
	}
	var v any
	if err := json.Unmarshal(data, &v); err != nil {
		return
	}
	key := "config_file:" + root + ":" + relPath
	meta := map[string]string{"format": "json"}
	switch t := v.(type) {
	case []any:
		meta["rows"] = fmt.Sprint(len(t))
		if len(t) > 0 {
			if obj, ok := t[0].(map[string]any); ok {
				var keys []string
				for k := range obj {
					keys = append(keys, k)
				}
				meta["headers"] = strings.Join(keys, ",")
				meta["columns"] = fmt.Sprint(len(keys))
			}
		}
	case map[string]any:
		var keys []string
		for k, val := range t {
			keys = append(keys, k)
			if arr, ok := val.([]any); ok {
				meta["rows"] = fmt.Sprint(len(arr))
			}
		}
		meta["headers"] = strings.Join(keys, ",")
		meta["columns"] = fmt.Sprint(len(keys))
	}
	res.Assets = append(res.Assets, model.Asset{
		Key: key, Root: root, Kind: "config_table", Path: relPath,
		Name: strings.TrimSuffix(path.Base(relPath), path.Ext(relPath)),
		Meta: meta,
	})
}

// DetectConfigReaders finds hot-reload capable config readers in the symbol table.
// This is intentionally data-driven: the caller supplies symbol (name, qualname)
// pairs and we look for the naming conventions used by the project.
func DetectConfigReaders(symbolNames map[string][]string) (readers []string, hotReload bool) {
	for _, pattern := range []string{"ConfigHotReloadManager", "ReloadConfig", "Config233AutoExportWatcher", "Config233Exporter"} {
		if _, ok := symbolNames[pattern]; ok {
			hotReload = true
			readers = append(readers, pattern)
		}
	}
	if _, ok := symbolNames["GetDataById"]; ok {
		readers = append(readers, "GetDataById")
	}
	if _, ok := symbolNames["GetUIController"]; ok {
		// not config related; ignore
	}
	return readers, hotReload
}

// filepathWalk is a bounded directory walker used by config discovery.
func filepathWalk(dir string, fn func(string), maxDepth int) error {
	base := dir
	return filepath.WalkDir(dir, func(p string, d os.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if d.IsDir() {
			name := d.Name()
			if p != base {
				for _, skip := range []string{".git", "node_modules", "Library", "Temp", "obj", "Logs", "Bundles"} {
					if name == skip {
						return filepath.SkipDir
					}
				}
			}
			rel, _ := filepath.Rel(base, p)
			if strings.Count(rel, string(os.PathSeparator)) >= maxDepth {
				return filepath.SkipDir
			}
			return nil
		}
		fn(p)
		return nil
	})
}
