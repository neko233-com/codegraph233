package config

import "strings"

// Features is the resolved capability matrix for one indexing run.  Everything
// is opt-in by profile and can be overridden field by field, so a project with
// no TypeScript / no assets never pays for those scanners.
type Features struct {
	Code         bool `json:"code"`
	Prefabs      bool `json:"prefabs"`
	Shaders      bool `json:"shaders"`
	Assets       bool `json:"assets"`
	Meta         bool `json:"meta"`
	ConfigTables bool `json:"config_tables"`
	GuidMap      bool `json:"guid_map"`
	TypeDeps     bool `json:"type_deps"`
}

// FeatureSet is the JSON shape: nil means "inherit from profile".
type FeatureSet struct {
	Code         *bool `json:"code,omitempty"`
	Prefabs      *bool `json:"prefabs,omitempty"`
	Shaders      *bool `json:"shaders,omitempty"`
	Assets       *bool `json:"assets,omitempty"`
	Meta         *bool `json:"meta,omitempty"`
	ConfigTables *bool `json:"config_tables,omitempty"`
	GuidMap      *bool `json:"guid_map,omitempty"`
	TypeDeps     *bool `json:"type_deps,omitempty"`
}

// Profile presets.  `full` is the historical behavior.
const (
	ProfileFull    = "full"
	ProfileGame    = "game"    // code + prefab/shader/assets + config, no meta scan
	ProfileCode    = "code"    // code only
	ProfileMinimal = "minimal" // code + prefabs
)

// Profiles lists the supported presets for `init` / docs.
var Profiles = []string{ProfileFull, ProfileGame, ProfileCode, ProfileMinimal}

// ResolveFeatures merges profile defaults with explicit overrides.
func (c *Config) ResolveFeatures() Features {
	profile := strings.ToLower(strings.TrimSpace(c.Profile))
	if profile == "" {
		profile = ProfileFull
	}
	var f Features
	switch profile {
	case ProfileCode:
		f = Features{Code: true, TypeDeps: true}
	case ProfileMinimal:
		f = Features{Code: true, TypeDeps: true, Prefabs: true}
	case ProfileGame:
		f = Features{Code: true, TypeDeps: true, Prefabs: true, Shaders: true, Assets: true, ConfigTables: true, GuidMap: true}
	default: // full
		f = Features{Code: true, TypeDeps: true, Prefabs: true, Shaders: true, Assets: true, Meta: true, ConfigTables: true, GuidMap: true}
	}
	set := func(dst *bool, v *bool) {
		if v != nil {
			*dst = *v
		}
	}
	set(&f.Code, c.Features.Code)
	set(&f.Prefabs, c.Features.Prefabs)
	set(&f.Shaders, c.Features.Shaders)
	set(&f.Assets, c.Features.Assets)
	set(&f.Meta, c.Features.Meta)
	set(&f.ConfigTables, c.Features.ConfigTables)
	set(&f.GuidMap, c.Features.GuidMap)
	set(&f.TypeDeps, c.Features.TypeDeps)
	return f
}

// EnabledCount reports how many capabilities are on (for status output).
func (f Features) EnabledCount() int {
	n := 0
	for _, v := range []bool{f.Code, f.TypeDeps, f.Prefabs, f.Shaders, f.Assets, f.Meta, f.ConfigTables, f.GuidMap} {
		if v {
			n++
		}
	}
	return n
}
