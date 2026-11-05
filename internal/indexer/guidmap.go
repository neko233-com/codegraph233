package indexer

import (
	"bufio"
	"encoding/csv"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"github.com/neko233-com/codegraph233/internal/config"
	"github.com/neko233-com/codegraph233/internal/store"
)

// LoadGuidMaps reads every configured guid->path mapping file.  Formats:
//
//	TSV:  guid<TAB>path<TAB>type   (header optional)
//	CSV:  guid,path,type           (header optional)
//	JSON: [{"guid":"...","path":"Assets/...","type":"Texture2D"}]
//
// Later files win on duplicate guids.  Only 32-hex guids are accepted because
// prefab/scene YAML references are serialized in that form.
func LoadGuidMaps(cfg *config.Config) ([]store.GuidRow, []string, error) {
	files := cfg.GuidMapFiles()
	byGUID := map[string]store.GuidRow{}
	var order []string
	var errs []string
	for _, f := range files {
		rows, err := parseGuidMapFile(f)
		if err != nil {
			errs = append(errs, fmt.Sprintf("%s: %v", filepath.Base(f), err))
			continue
		}
		for _, r := range rows {
			r.GUID = strings.ToLower(strings.TrimSpace(r.GUID))
			if !isHex32(r.GUID) || r.Path == "" {
				continue
			}
			if _, ok := byGUID[r.GUID]; !ok {
				order = append(order, r.GUID)
			}
			byGUID[r.GUID] = r
		}
	}
	out := make([]store.GuidRow, 0, len(order))
	for _, g := range order {
		out = append(out, byGUID[g])
	}
	if len(errs) > 0 && len(out) == 0 {
		return nil, files, fmt.Errorf("%s", strings.Join(errs, "; "))
	}
	return out, files, nil
}

func parseGuidMapFile(path string) ([]store.GuidRow, error) {
	ext := strings.ToLower(filepath.Ext(path))
	switch ext {
	case ".json":
		return parseGuidJSON(path)
	case ".csv":
		return parseGuidDelimited(path, ',')
	case ".tsv", ".txt", ".tab":
		return parseGuidDelimited(path, '\t')
	}
	return nil, fmt.Errorf("unsupported guid map format %q", ext)
}

func parseGuidDelimited(path string, sep rune) ([]store.GuidRow, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()
	r := csv.NewReader(bufio.NewReaderSize(f, 1<<20))
	r.Comma = sep
	r.FieldsPerRecord = -1
	r.LazyQuotes = true
	var out []store.GuidRow
	first := true
	for {
		rec, err := r.Read()
		if err != nil {
			if err == io.EOF {
				break
			}
			return out, err
		}
		if len(rec) == 0 {
			continue
		}
		if first {
			first = false
			if !isHex32(strings.TrimSpace(rec[0])) {
				continue // header row
			}
		}
		row := store.GuidRow{GUID: strings.TrimSpace(rec[0])}
		if len(rec) > 1 {
			row.Path = strings.TrimSpace(rec[1])
		}
		if len(rec) > 2 {
			row.Type = strings.TrimSpace(rec[2])
		}
		if row.GUID != "" && row.Path != "" {
			out = append(out, row)
		}
	}
	return out, nil
}

func parseGuidJSON(path string) ([]store.GuidRow, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var raw []struct {
		GUID string `json:"guid"`
		Path string `json:"path"`
		Type string `json:"type"`
	}
	if err := json.Unmarshal(data, &raw); err != nil {
		return nil, err
	}
	out := make([]store.GuidRow, 0, len(raw))
	for _, r := range raw {
		if r.GUID != "" && r.Path != "" {
			out = append(out, store.GuidRow{GUID: r.GUID, Path: r.Path, Type: r.Type})
		}
	}
	return out, nil
}

func isHex32(s string) bool {
	if len(s) != 32 {
		return false
	}
	for i := 0; i < 32; i++ {
		c := s[i]
		if !((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F')) {
			return false
		}
	}
	return true
}
