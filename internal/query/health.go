package query

import (
	"encoding/json"
	"os"
	"strings"

	"github.com/neko233-com/codegraph233/internal/config"
)

// OrphanAssets lists resource-like assets that nothing references.  Entry
// points (prefab/scene/shader/config) are excluded by default because they are
// often loaded by name at runtime.
func (e *Engine) OrphanAssets(root string, kinds []string, limit int) ([]AssetHit, error) {
	if limit <= 0 {
		limit = 200
	}
	if len(kinds) == 0 {
		kinds = []string{
			"image", "audio", "video", "font", "tmp_font", "spine_data", "spine_atlas",
			"spine_json", "material", "animation_clip", "animator_controller",
			"audio_mixer", "timeline", "sprite_atlas", "scriptable_asset",
		}
	}
	args := []any{}
	ph := placeholders(len(kinds))
	for _, k := range kinds {
		args = append(args, k)
	}
	q := `SELECT a.asset_key, a.kind, COALESCE(a.root,''), COALESCE(a.path,''), COALESCE(a.name,''),
		COALESCE(a.unity_id,''), COALESCE(a.line,0), COALESCE(a.meta,'{}')
		FROM assets a
		WHERE a.path <> ''
		  AND a.kind IN (` + ph + `)
		  AND NOT EXISTS (SELECT 1 FROM asset_edges e WHERE e.dst = a.asset_key)`
	if root != "" {
		q += ` AND a.root = ?`
		args = append(args, root)
	}
	q += ` ORDER BY CAST(COALESCE(json_extract(a.meta,'$.bytes'),'0') AS INTEGER) DESC LIMIT ?`
	args = append(args, limit)
	rows, err := e.St.DB().Query(q, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []AssetHit{}
	for rows.Next() {
		var h AssetHit
		var meta string
		if err := rows.Scan(&h.Key, &h.Kind, &h.Root, &h.Path, &h.Name, &h.UnityID, &h.Line, &meta); err != nil {
			return nil, err
		}
		_ = json.Unmarshal([]byte(meta), &h.Meta)
		out = append(out, h)
	}
	return out, rows.Err()
}

// Health is a single-call project/index health snapshot.
type Health struct {
	Files          int             `json:"files"`
	Symbols        int             `json:"symbols"`
	Refs           int             `json:"refs"`
	Edges          int             `json:"edges"`
	UnresolvedRefs int             `json:"unresolved_refs"`
	Assets         int             `json:"assets"`
	AssetEdges     int             `json:"asset_edges"`
	OrphanAssets   int             `json:"orphan_assets"`
	LinkedAssets   int             `json:"linked_assets"`
	GuidEntries    int             `json:"guid_map_entries"`
	GuidRefs       int             `json:"guid_refs"`
	GuidMapped     int             `json:"guid_refs_mapped"`
	GuidCoverage   string          `json:"guid_coverage"`
	DbBytes        int64           `json:"db_bytes"`
	IndexedAt      string          `json:"indexed_at,omitempty"`
	Profile        string          `json:"profile,omitempty"`
	Features       config.Features `json:"features"`
	Largest        []AssetSize     `json:"largest_assets,omitempty"`
}

// HealthReport aggregates counts, guid coverage and the largest assets.
func (e *Engine) HealthReport(root string) (*Health, error) {
	h := &Health{Profile: e.Cfg.Profile, Features: e.Cfg.ResolveFeatures()}
	db := e.St.DB()
	_ = db.QueryRow(`SELECT (SELECT COUNT(*) FROM files), (SELECT COUNT(*) FROM symbols),
		(SELECT COUNT(*) FROM refs), (SELECT COUNT(*) FROM edges),
		(SELECT COUNT(*) FROM refs WHERE COALESCE(target,0)=0),
		(SELECT COUNT(*) FROM assets), (SELECT COUNT(*) FROM asset_edges),
		(SELECT COUNT(*) FROM guid_map)`).Scan(
		&h.Files, &h.Symbols, &h.Refs, &h.Edges, &h.UnresolvedRefs, &h.Assets, &h.AssetEdges, &h.GuidEntries)
	h.OrphanAssets, h.LinkedAssets = e.St.AssetHealthCounts()
	_ = db.QueryRow(`SELECT COUNT(*) FROM (SELECT DISTINCT dst FROM asset_edges WHERE dst LIKE 'guid:%')`).Scan(&h.GuidRefs)
	_ = db.QueryRow(`SELECT COUNT(*) FROM (SELECT DISTINCT dst FROM asset_edges e
		WHERE dst LIKE 'guid:%' AND EXISTS (SELECT 1 FROM guid_map g WHERE g.guid = SUBSTR(e.dst, 6)))`).Scan(&h.GuidMapped)
	if h.GuidRefs > 0 {
		h.GuidCoverage = formatPct(h.GuidMapped, h.GuidRefs)
	}
	h.IndexedAt = e.St.GetMeta("indexed_at")
	if st, err := os.Stat(e.Cfg.DBPath()); err == nil {
		h.DbBytes = st.Size()
	}
	if stats, err := e.AssetStats(root, 1); err == nil {
		for _, s := range stats {
			if len(s.Largest) > 0 {
				h.Largest = append(h.Largest, s.Largest[0])
			}
		}
	}
	return h, nil
}

func formatPct(a, b int) string {
	if b == 0 {
		return "n/a"
	}
	return strings.TrimRight(strings.TrimRight(
		formatFloat(float64(a)*100/float64(b)), "0"), ".") + "%"
}

func formatFloat(v float64) string {
	// one decimal, no fmt dependency in hot paths
	whole := int(v)
	frac := int((v - float64(whole)) * 10)
	if frac < 0 {
		frac = -frac
	}
	return itoaLocal(whole) + "." + itoaLocal(frac)
}

func itoaLocal(v int) string {
	if v == 0 {
		return "0"
	}
	neg := v < 0
	if neg {
		v = -v
	}
	var b [20]byte
	i := len(b)
	for v > 0 {
		i--
		b[i] = byte('0' + v%10)
		v /= 10
	}
	if neg {
		i--
		b[i] = '-'
	}
	return string(b[i:])
}
