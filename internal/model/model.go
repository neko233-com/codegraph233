// Package model holds the shared graph records produced by every front-end.
package model

// Symbol is a declaration node: namespace, type, member, function, asset or config table.
type Symbol struct {
	ID        int64
	FileID    int64
	Root      string
	Lang      string
	Kind      string
	Name      string
	Qualname  string
	Namespace string
	ParentID  int64
	StartLine int
	EndLine   int
	Signature string
	Modifiers string
	// ParamCount is -1 when the language/front-end cannot determine it.
	ParamCount int
	Container  string
	Module     string
}

// Ref is an unresolved reference harvested from a body; the binder sets Target.
type Ref struct {
	ID            int64
	SrcID         int64
	SrcKey        string
	FileID        int64
	Root          string
	Name          string
	Receiver      string
	ReceiverType  string
	Kind          string
	NamespaceHint string
	Line          int
	Arity         int
	Target        int64
	Confidence    string
}

// FileRec is one indexed source/asset file.
type FileRec struct {
	ID      int64
	Root    string
	Path    string // root-relative, slash separated
	Lang    string
	Module  string
	Size    int64
	ModTime int64 // unix nanoseconds
}

// FileResult is the output of a language front-end for one file.
type FileResult struct {
	RelPath string
	Root    string
	Lang    string
	Module  string
	Symbols []Symbol
	Refs    []Ref
	// Assets are language/framework specific nodes (prefab objects, shader passes).
	Assets []Asset
	// AssetEdges connect assets by stable asset keys.
	AssetEdges []AssetEdge
	Errors     []string
}

// Asset is a non-code graph node such as a prefab, scene, shader, texture or config table.
type Asset struct {
	Key    string // stable identity within the index, e.g. prefab:<root>:<path>
	Root   string
	Kind   string // prefab, prefab_object, scene, texture, material, audio, shader, shader_pass, config_table, config_source
	Path   string // project-relative path when the asset lives in a file
	Name   string
	FileID string // Unity fileID inside the YAML document, when relevant
	Line   int
	Meta   map[string]string
}

// AssetEdge connects two assets (or an asset and a symbol) by key.
type AssetEdge struct {
	Src      string
	Dst      string
	Kind     string // contains, instantiates, references, uses_script, includes, reads_config
	Line     int
	Detail   string
	Root     string
}

// ResolvedEdge is a graph edge between two symbols.
type ResolvedEdge struct {
	Src        int64
	Dst        int64
	Kind       string
	Line       int
	Confidence string
}

// IndexStats summarizes one indexing run.
type IndexStats struct {
	Roots        int            `json:"roots"`
	Files        int            `json:"files"`
	ChangedFiles int            `json:"changed_files"`
	RemovedFiles int            `json:"removed_files"`
	Symbols      int            `json:"symbols"`
	Refs         int            `json:"refs"`
	Edges        int            `json:"edges"`
	Assets       int            `json:"assets"`
	AssetEdges   int            `json:"asset_edges"`
	Unresolved   int            `json:"unresolved_refs"`
	Errors       []string       `json:"errors,omitempty"`
	ByLang       map[string]int `json:"files_by_lang"`
	ByRoot       map[string]int `json:"files_by_root"`
	ElapsedMs    int64          `json:"elapsed_ms"`
}

// Strategy describes how the project was recognized.
type Strategy struct {
	Name       string   `json:"name"` // unity, tuanjie, ue, generic
	Confidence string   `json:"confidence"`
	Evidence   []string `json:"evidence,omitempty"`
	Engine     string   `json:"engine,omitempty"`
	Languages  []string `json:"languages,omitempty"`
	Notes      string   `json:"notes,omitempty"`
}

const (
	KindNamespace  = "namespace"
	KindPackage    = "package"
	KindClass      = "class"
	KindStruct     = "struct"
	KindInterface  = "interface"
	KindEnum       = "enum"
	KindDelegate   = "delegate"
	KindRecord     = "record"
	KindType       = "type"
	KindMethod     = "method"
	KindConstructor = "constructor"
	KindFunction   = "function"
	KindProperty   = "property"
	KindField      = "field"
	KindEvent      = "event"
	KindEnumMember = "enum_member"
	KindOperator   = "operator"
	KindShaderFunc = "shader_func"
)

// IsTypeKind reports whether kind declares a type (used for inheritance binding).
func IsTypeKind(kind string) bool {
	switch kind {
	case KindClass, KindStruct, KindInterface, KindEnum, KindDelegate, KindRecord, KindType:
		return true
	}
	return false
}
