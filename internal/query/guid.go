package query

import (
	"database/sql"
	"fmt"
	"strings"
)

// GuidRef is one resolved guid -> asset mapping.
type GuidRef struct {
	GUID string `json:"guid"`
	Path string `json:"path"`
	Type string `json:"type,omitempty"`
	Name string `json:"name,omitempty"`
}

// GuidHit is the answer of a guid/path lookup.
type GuidHit struct {
	GuidRef
	ReferencedBy []AssetEdgeHit `json:"referenced_by,omitempty"`
	Referenced   []AssetEdgeHit `json:"referenced,omitempty"`
}

// lookupGuids batches guid resolution for edge rendering.
func (e *Engine) lookupGuids(guids []string) map[string]GuidRef {
	out := map[string]GuidRef{}
	if len(guids) == 0 {
		return out
	}
	uniq := make([]string, 0, len(guids))
	seen := map[string]bool{}
	for _, g := range guids {
		if g == "" || seen[g] {
			continue
		}
		seen[g] = true
		uniq = append(uniq, g)
	}
	const batch = 400
	for start := 0; start < len(uniq); start += batch {
		end := start + batch
		if end > len(uniq) {
			end = len(uniq)
		}
		q := `SELECT guid, path, COALESCE(type,''), COALESCE(name,'') FROM guid_map WHERE guid IN (` +
			placeholders(end-start) + `)`
		args := make([]any, 0, end-start)
		for _, g := range uniq[start:end] {
			args = append(args, g)
		}
		rows, err := e.St.DB().Query(q, args...)
		if err != nil {
			continue
		}
		for rows.Next() {
			var r GuidRef
			if rows.Scan(&r.GUID, &r.Path, &r.Type, &r.Name) == nil {
				out[r.GUID] = r
			}
		}
		rows.Close()
	}
	return out
}

func guidOf(key string) string {
	if strings.HasPrefix(key, "guid:") {
		return strings.TrimPrefix(key, "guid:")
	}
	return ""
}

// LookupGuid resolves a guid (32 hex) or a path substring to asset info plus
// its incoming/outgoing references.
func (e *Engine) LookupGuid(guidOrPath string, limit int) ([]GuidHit, error) {
	if limit <= 0 {
		limit = 100
	}
	key := strings.ToLower(strings.TrimSpace(guidOrPath))
	key = strings.TrimPrefix(key, "guid:")
	var rows *sql.Rows
	var err error
	if isHex32(key) {
		rows, err = e.St.DB().Query(`SELECT guid, path, COALESCE(type,''), COALESCE(name,'')
			FROM guid_map WHERE guid = ? LIMIT 1`, key)
	} else {
		rows, err = e.St.DB().Query(`SELECT guid, path, COALESCE(type,''), COALESCE(name,'')
			FROM guid_map WHERE path LIKE ? ORDER BY LENGTH(path) LIMIT ?`, "%"+guidOrPath+"%", limit)
	}
	if err != nil {
		return nil, err
	}
	var hits []GuidHit
	for rows.Next() {
		var h GuidHit
		if err := rows.Scan(&h.GUID, &h.Path, &h.Type, &h.Name); err == nil {
			hits = append(hits, h)
		}
	}
	rows.Close()
	if len(hits) == 0 {
		return nil, fmt.Errorf("no guid mapping matched %q（先运行编辑器导出：Tools/codegraph233/导出 GUID 映射）", guidOrPath)
	}
	for i := range hits {
		hits[i].ReferencedBy, _ = e.AssetEdges("guid:"+hits[i].GUID, "in", limit)
		hits[i].Referenced, _ = e.AssetEdges("guid:"+hits[i].GUID, "out", limit)
	}
	return hits, nil
}

// GuidCoverage reports how many distinct guid references resolve to an asset.
func (e *Engine) GuidCoverage() (mapped, total, entries int, err error) {
	row := e.St.DB().QueryRow(`SELECT COUNT(*) FROM guid_map`)
	if err = row.Scan(&entries); err != nil {
		return 0, 0, 0, err
	}
	row = e.St.DB().QueryRow(`SELECT COUNT(*) FROM (SELECT DISTINCT dst FROM asset_edges WHERE dst LIKE 'guid:%')`)
	if err = row.Scan(&total); err != nil {
		return 0, 0, entries, err
	}
	row = e.St.DB().QueryRow(`SELECT COUNT(*) FROM (SELECT DISTINCT dst FROM asset_edges e
		WHERE dst LIKE 'guid:%' AND EXISTS (SELECT 1 FROM guid_map g WHERE g.guid = SUBSTR(e.dst, 6)))`)
	if err = row.Scan(&mapped); err != nil {
		return 0, total, entries, err
	}
	return mapped, total, entries, nil
}

func isHex32(s string) bool {
	if len(s) != 32 {
		return false
	}
	for i := 0; i < 32; i++ {
		c := s[i]
		if !((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f')) {
			return false
		}
	}
	return true
}
