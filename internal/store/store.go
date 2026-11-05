// Package store persists the graph in a local SQLite database (pure Go driver,
// no cgo) so that indexing is incremental and queries stay fast on 20k+ files.
package store

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	_ "modernc.org/sqlite"

	"github.com/neko233-com/codegraph233/internal/model"
)

// Store wraps the index database.
type Store struct {
	db   *sql.DB
	path string
}

// Open opens (creating if required) the index database in the safe (WAL)
// profile, suitable for concurrent readers.
func Open(path string) (*Store, error) { return openWith(path, false) }

// OpenFast opens the database for bulk indexing: in-memory journal and
// synchronous=OFF remove fsync cost.  A crash can lose the index, which is
// always rebuildable, so callers use this only for write-heavy runs.
func OpenFast(path string) (*Store, error) { return openWith(path, true) }

// OpenFastLite is the low-memory write profile: smaller page cache and mmap
// window, trading some throughput for a lower resident set.
func OpenFastLite(path string) (*Store, error) { return openWith(path, true) }

var liteMode bool

// SetLite switches the fast profile to smaller caches (set before OpenFast).
func SetLite(on bool) { liteMode = on }

func openWith(path string, fast bool) (*Store, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return nil, err
	}
	pragmas := "?_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)&_pragma=busy_timeout(8000)&_pragma=temp_store(MEMORY)&_pragma=cache_size(-32000)"
	if fast {
		// journal_mode=OFF removes the rollback journal entirely: the closest
		// SQLite gets to "no transaction overhead".  The index is rebuildable, so
		// a crash during a run only costs a re-index.
		pragmas = "?_pragma=journal_mode(OFF)&_pragma=synchronous(OFF)&_pragma=busy_timeout(8000)" +
			"&_pragma=temp_store(MEMORY)&_pragma=cache_size(-131072)&_pragma=mmap_size(268435456)"
	}
	dsn := "file:" + filepath.ToSlash(path) + pragmas
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	if err := db.Ping(); err != nil {
		db.Close()
		return nil, err
	}
	s := &Store{db: db, path: path}
	if err := s.Migrate(); err != nil {
		db.Close()
		return nil, err
	}
	return s, nil
}

// DB exposes the underlying handle for the query package.
func (s *Store) DB() *sql.DB { return s.db }

// Path is the database file path.
func (s *Store) Path() string { return s.path }

// Close checkpoints the WAL (so the index directory stays compact) and closes
// the database.
func (s *Store) Close() error {
	if _, err := s.db.Exec(`PRAGMA wal_checkpoint(TRUNCATE)`); err != nil {
		_ = err
	}
	return s.db.Close()
}

const schema = `
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT
);
CREATE TABLE IF NOT EXISTS files (
  id INTEGER PRIMARY KEY,
  root TEXT NOT NULL,
  path TEXT NOT NULL,
  lang TEXT NOT NULL,
  module TEXT,
  size INTEGER NOT NULL,
  mtime INTEGER NOT NULL,
  UNIQUE(root, path)
);
CREATE TABLE IF NOT EXISTS symbols (
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL,
  root TEXT NOT NULL,
  lang TEXT NOT NULL,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  qualname TEXT NOT NULL,
  namespace TEXT,
  parent_id INTEGER DEFAULT 0,
  start_line INTEGER,
  end_line INTEGER,
  signature TEXT,
  modifiers TEXT,
  param_count INTEGER DEFAULT -1,
  container TEXT,
  module TEXT
);
CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_qual ON symbols(qualname);
CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_id);
CREATE INDEX IF NOT EXISTS idx_symbols_container ON symbols(container);
CREATE INDEX IF NOT EXISTS idx_symbols_lang ON symbols(lang);
CREATE TABLE IF NOT EXISTS refs (
  id INTEGER PRIMARY KEY,
  src_id INTEGER NOT NULL DEFAULT 0,
  src_key TEXT,
  file_id INTEGER NOT NULL,
  root TEXT,
  name TEXT NOT NULL,
  receiver TEXT,
  receiver_type TEXT,
  kind TEXT NOT NULL,
  namespace_hint TEXT,
  line INTEGER,
  arity INTEGER DEFAULT -1,
  target INTEGER DEFAULT 0,
  confidence TEXT
);
CREATE INDEX IF NOT EXISTS idx_refs_target ON refs(target);
CREATE INDEX IF NOT EXISTS idx_refs_file ON refs(file_id);
CREATE INDEX IF NOT EXISTS idx_refs_name ON refs(name);
CREATE TABLE IF NOT EXISTS edges (
  src INTEGER NOT NULL,
  dst INTEGER NOT NULL,
  kind TEXT NOT NULL,
  line INTEGER,
  confidence TEXT
);
CREATE INDEX IF NOT EXISTS idx_edges_src ON edges(src);
CREATE INDEX IF NOT EXISTS idx_edges_dst ON edges(dst);
CREATE TABLE IF NOT EXISTS assets (
  id INTEGER PRIMARY KEY,
  asset_key TEXT NOT NULL UNIQUE,
  root TEXT,
  kind TEXT NOT NULL,
  path TEXT,
  name TEXT,
  unity_id TEXT,
  line INTEGER,
  meta TEXT
);
CREATE INDEX IF NOT EXISTS idx_assets_path ON assets(path);
CREATE INDEX IF NOT EXISTS idx_assets_kind ON assets(kind);
CREATE INDEX IF NOT EXISTS idx_assets_name ON assets(name);
CREATE TABLE IF NOT EXISTS asset_edges (
  src TEXT NOT NULL,
  dst TEXT NOT NULL,
  kind TEXT NOT NULL,
  line INTEGER,
  detail TEXT,
  root TEXT
);
CREATE INDEX IF NOT EXISTS idx_asset_edges_src ON asset_edges(src);
CREATE INDEX IF NOT EXISTS idx_asset_edges_dst ON asset_edges(dst);
CREATE TABLE IF NOT EXISTS guid_map (
  guid TEXT PRIMARY KEY,
  path TEXT NOT NULL,
  name TEXT,
  type TEXT
);
CREATE INDEX IF NOT EXISTS idx_guid_map_path ON guid_map(path);
`

// Migrate creates the schema and stamps the schema version.
func (s *Store) Migrate() error {
	if _, err := s.db.Exec(schema); err != nil {
		return fmt.Errorf("migrate: %w", err)
	}
	// asset_edges must be unique so guid materialization is idempotent.  The
	// dedupe scan is only needed on the first migration (it is O(table)).
	var idxCount int
	_ = s.db.QueryRow(`SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_asset_edges_uniq'`).Scan(&idxCount)
	if idxCount == 0 {
		if _, err := s.db.Exec(`DELETE FROM asset_edges WHERE rowid NOT IN
			(SELECT MIN(rowid) FROM asset_edges GROUP BY src, dst, kind, COALESCE(detail,''))`); err != nil {
			return fmt.Errorf("migrate asset edges: %w", err)
		}
		if _, err := s.db.Exec(`CREATE UNIQUE INDEX IF NOT EXISTS idx_asset_edges_uniq
			ON asset_edges(src, dst, kind, detail)`); err != nil {
			return fmt.Errorf("migrate asset edge index: %w", err)
		}
	}
	return s.SetMeta("schema_version", "2")
}

// MaterializeGuidEdges adds asset-to-asset edges for every guid endpoint that
// has a mapping, so queries do not have to resolve guids per row.  Idempotent.
func (s *Store) MaterializeGuidEdges() (int, error) {
	res, err := s.db.Exec(`INSERT OR IGNORE INTO asset_edges(src, dst, kind, line, detail, root)
		SELECT e.src,
		       'asset:' || COALESCE((SELECT a.root FROM assets a WHERE a.asset_key = e.src), e.root, '') || ':' || g.path,
		       e.kind, e.line, e.detail, e.root
		FROM asset_edges e
		JOIN guid_map g ON g.guid = SUBSTR(e.dst, 6)
		WHERE e.dst LIKE 'guid:%'`)
	if err != nil {
		return 0, err
	}
	n, _ := res.RowsAffected()
	return int(n), nil
}

// AssetHealthCounts returns (orphan candidates, assets with a path).
func (s *Store) AssetHealthCounts() (orphans int, linked int) {
	_ = s.db.QueryRow(`SELECT COUNT(*) FROM assets a
		WHERE a.path <> '' AND a.kind NOT IN ('guid_ref','prefab_object','scene_object','shader_property','shader_pass','config_source')
		  AND NOT EXISTS (SELECT 1 FROM asset_edges e WHERE e.dst = a.asset_key)`).Scan(&orphans)
	_ = s.db.QueryRow(`SELECT COUNT(*) FROM assets a
		WHERE a.path <> '' AND EXISTS (SELECT 1 FROM asset_edges e WHERE e.dst = a.asset_key)`).Scan(&linked)
	return
}

// FileState is the incremental scan fingerprint of one file.
type FileState struct {
	ID      int64
	Size    int64
	ModTime int64
	Lang    string
}

// LoadFileStates returns root|path -> state.
func (s *Store) LoadFileStates() (map[string]FileState, error) {
	rows, err := s.db.Query(`SELECT id, root, path, size, mtime, lang FROM files`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := map[string]FileState{}
	for rows.Next() {
		var id, size, mtime int64
		var root, path, lang string
		if err := rows.Scan(&id, &root, &path, &size, &mtime, &lang); err != nil {
			return nil, err
		}
		out[root+"|"+path] = FileState{ID: id, Size: size, ModTime: mtime, Lang: lang}
	}
	return out, rows.Err()
}

// Tx is a write transaction with reusable statements.
type Tx struct {
	db           *sql.DB
	mode         string
	tx           *sql.Tx
	delSym       *sql.Stmt
	delRefs      *sql.Stmt
	delAssets    *sql.Stmt
	delAssetEd   *sql.Stmt
	insSymbol    *sql.Stmt
	insRef       *sql.Stmt
	insAsset     *sql.Stmt
	insAssetEdge *sql.Stmt
	insGuid      *sql.Stmt
}

// Begin starts a write transaction (batch mode).
func (s *Store) Begin() (*Tx, error) { return s.BeginMode("batch") }

// BeginMode starts a write session:
//   - batch  : explicit transaction, caller commits every N files
//   - single : one transaction for the whole run (caller commits at the end)
//   - none   : autocommit, no transaction at all (each statement commits)
//
// In `none` mode statements still go through prepared statements on the pool so
// the only difference versus `batch` is transactionality itself.
func (s *Store) BeginMode(mode string) (*Tx, error) {
	t := &Tx{db: s.db, mode: mode}
	var err error
	prepare := func(q string) (*sql.Stmt, error) {
		if mode == "none" {
			return s.db.Prepare(q)
		}
		return t.tx.Prepare(q)
	}
	if mode != "none" {
		if t.tx, err = s.db.Begin(); err != nil {
			return nil, err
		}
	}
	t.delSym, err = prepare(`DELETE FROM symbols WHERE file_id = ?`)
	if err != nil {
		return nil, err
	}
	t.delRefs, err = prepare(`DELETE FROM refs WHERE file_id = ?`)
	if err != nil {
		return nil, err
	}
	t.delAssets, err = prepare(`DELETE FROM assets WHERE path = ?`)
	if err != nil {
		return nil, err
	}
	t.delAssetEd, err = prepare(`DELETE FROM asset_edges WHERE src IN (SELECT asset_key FROM assets WHERE path = ?) OR dst IN (SELECT asset_key FROM assets WHERE path = ?)`)
	if err != nil {
		return nil, err
	}
	if t.insSymbol, err = prepare(`INSERT INTO symbols
		(file_id, root, lang, kind, name, qualname, namespace, parent_id, start_line, end_line,
		 signature, modifiers, param_count, container, module)
		VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)`); err != nil {
		return nil, err
	}
	if t.insRef, err = prepare(`INSERT INTO refs
		(src_id, src_key, file_id, root, name, receiver, receiver_type, kind, namespace_hint, line, arity)
		VALUES(?,?,?,?,?,?,?,?,?,?,?)`); err != nil {
		return nil, err
	}
	if t.insAsset, err = prepare(`INSERT INTO assets(asset_key, root, kind, path, name, unity_id, line, meta)
		VALUES(?,?,?,?,?,?,?,?)
		ON CONFLICT(asset_key) DO UPDATE SET root=excluded.root,
		  kind = CASE WHEN excluded.path <> '' THEN excluded.kind ELSE assets.kind END,
		  path = CASE WHEN excluded.path <> '' THEN excluded.path ELSE assets.path END,
		  name = CASE WHEN excluded.name <> '' THEN excluded.name ELSE assets.name END,
		  unity_id = COALESCE(NULLIF(excluded.unity_id,''), assets.unity_id),
		  line=excluded.line, meta=excluded.meta`); err != nil {
		return nil, err
	}
	if t.insAssetEdge, err = prepare(`INSERT OR IGNORE INTO asset_edges(src, dst, kind, line, detail, root) VALUES(?,?,?,?,?,?)`); err != nil {
		return nil, err
	}
	if t.insGuid, err = prepare(`INSERT OR IGNORE INTO guid_map(guid, path, name, type) VALUES(?,?,?,?)`); err != nil {
		return nil, err
	}
	return t, nil
}

func (t *Tx) exec(q string, args ...any) (sql.Result, error) {
	if t.tx != nil {
		return t.tx.Exec(q, args...)
	}
	return t.db.Exec(q, args...)
}

func (t *Tx) queryRow(q string, args ...any) *sql.Row {
	if t.tx != nil {
		return t.tx.QueryRow(q, args...)
	}
	return t.db.QueryRow(q, args...)
}

// Commit finalizes the transaction (no-op in `none` mode).
func (t *Tx) Commit() error {
	if t.tx == nil {
		return nil
	}
	return t.tx.Commit()
}

// Rollback aborts the transaction (no-op in `none` mode).
func (t *Tx) Rollback() error {
	if t.tx == nil {
		return nil
	}
	return t.tx.Rollback()
}

// Mode reports the write mode of this session.
func (t *Tx) Mode() string { return t.mode }

// AddGuidRows contributes plaintext-meta mappings without overriding the
// authoritative editor export (INSERT OR IGNORE).
func (t *Tx) AddGuidRows(rows []GuidRow) error {
	if len(rows) == 0 {
		return nil
	}
	stmt := t.insGuid
	for _, r := range rows {
		if _, err := stmt.Exec(r.GUID, r.Path, filepath.Base(r.Path), r.Type); err != nil {
			return err
		}
	}
	return nil
}

// FileID returns the row id of a file inside the transaction, or 0.
func (t *Tx) FileID(root, relPath string) (int64, error) {
	var id int64
	err := t.queryRow(`SELECT id FROM files WHERE root=? AND path=?`, root, relPath).Scan(&id)
	if err == sql.ErrNoRows {
		return 0, nil
	}
	return id, err
}

// DeleteFile removes a file and everything derived from it.
func (t *Tx) DeleteFile(fileID int64, relPath string) error {
	if _, err := t.exec(`DELETE FROM edges WHERE src IN (SELECT id FROM symbols WHERE file_id = ?)`, fileID); err != nil {
		return err
	}
	if _, err := t.delSym.Exec(fileID); err != nil {
		return err
	}
	if _, err := t.delRefs.Exec(fileID); err != nil {
		return err
	}
	if _, err := t.exec(`DELETE FROM files WHERE id = ?`, fileID); err != nil {
		return err
	}
	if relPath != "" {
		if _, err := t.delAssetEd.Exec(relPath, relPath); err != nil {
			return err
		}
		if _, err := t.delAssets.Exec(relPath); err != nil {
			return err
		}
	}
	return nil
}

// UpsertFile inserts or updates a file record and returns its id.
func (t *Tx) UpsertFile(f model.FileRec) (int64, error) {
	var id int64
	err := t.queryRow(`SELECT id FROM files WHERE root=? AND path=?`, f.Root, f.Path).Scan(&id)
	if err == nil {
		_, err = t.exec(`UPDATE files SET lang=?, module=?, size=?, mtime=? WHERE id=?`,
			f.Lang, f.Module, f.Size, f.ModTime, id)
		return id, err
	}
	if err != sql.ErrNoRows {
		return 0, err
	}
	res, err := t.exec(`INSERT INTO files(root, path, lang, module, size, mtime) VALUES(?,?,?,?,?,?)`,
		f.Root, f.Path, f.Lang, f.Module, f.Size, f.ModTime)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}

// InsertSymbols inserts symbols in order and returns their ids.
func (t *Tx) InsertSymbols(fileID int64, syms []model.Symbol) ([]int64, error) {
	stmt := t.insSymbol
	ids := make([]int64, len(syms))
	for i, s := range syms {
		res, err := stmt.Exec(fileID, s.Root, s.Lang, s.Kind, s.Name, s.Qualname, s.Namespace,
			s.ParentID, s.StartLine, s.EndLine, s.Signature, s.Modifiers, s.ParamCount, s.Container, s.Module)
		if err != nil {
			return nil, fmt.Errorf("insert symbol %s: %w", s.Qualname, err)
		}
		id, err := res.LastInsertId()
		if err != nil {
			return nil, err
		}
		ids[i] = id
		// parent linkage by qualname is resolved in a second pass by the indexer
	}
	return ids, nil
}

// UpdateSymbolParents fills parent_id using container qualnames.  The mapping is
// computed in Go and written through a temp table: a correlated subquery over
// the whole symbols table would be quadratic on a 200k-symbol project.
func (s *Store) UpdateSymbolParents(root string) error {
	rows, err := s.db.Query(`SELECT id, qualname, lang, COALESCE(container,''), kind FROM symbols WHERE root = ?`, root)
	if err != nil {
		return err
	}
	type key struct{ qual, lang string }
	typeIDs := map[key]int64{}
	type pending struct {
		id        int64
		container string
		lang      string
	}
	var all []pending
	for rows.Next() {
		var id int64
		var qual, lang, container, kind string
		if err := rows.Scan(&id, &qual, &lang, &container, &kind); err != nil {
			rows.Close()
			return err
		}
		all = append(all, pending{id: id, container: container, lang: lang})
		if model.IsTypeKind(kind) {
			k := key{qual, lang}
			if _, ok := typeIDs[k]; !ok {
				typeIDs[k] = id
			}
		}
	}
	rows.Close()
	if len(all) == 0 {
		return nil
	}
	type pair struct{ id, parent int64 }
	pairs := make([]pair, 0, len(all))
	for _, p := range all {
		if p.container == "" {
			continue
		}
		if pid, ok := typeIDs[key{p.container, p.lang}]; ok && pid != p.id {
			pairs = append(pairs, pair{p.id, pid})
		}
	}
	if len(pairs) == 0 {
		return nil
	}
	tx, err := s.db.Begin()
	if err != nil {
		return err
	}
	if _, err := tx.Exec(`CREATE TEMP TABLE IF NOT EXISTS _parents(id INTEGER PRIMARY KEY, parent INTEGER)`); err != nil {
		tx.Rollback()
		return err
	}
	if _, err := tx.Exec(`DELETE FROM _parents`); err != nil {
		tx.Rollback()
		return err
	}
	const batch = 800
	for start := 0; start < len(pairs); start += batch {
		end := start + batch
		if end > len(pairs) {
			end = len(pairs)
		}
		var b strings.Builder
		b.WriteString(`INSERT OR REPLACE INTO _parents(id,parent) VALUES `)
		args := make([]any, 0, (end-start)*2)
		for i := start; i < end; i++ {
			if i > start {
				b.WriteByte(',')
			}
			b.WriteString(`(?,?)`)
			args = append(args, pairs[i].id, pairs[i].parent)
		}
		if _, err := tx.Exec(b.String(), args...); err != nil {
			tx.Rollback()
			return err
		}
	}
	if _, err := tx.Exec(`UPDATE symbols SET parent_id = p.parent FROM _parents p WHERE symbols.id = p.id`); err != nil {
		tx.Rollback()
		return err
	}
	return tx.Commit()
}

// InsertRefs inserts references with their owning symbol id resolved from the
// per-file key map.
func (t *Tx) InsertRefs(fileID int64, refs []model.Ref, keyToID map[string]int64) error {
	stmt := t.insRef
	for _, r := range refs {
		srcID := keyToID[r.SrcKey]
		if _, err := stmt.Exec(srcID, r.SrcKey, fileID, r.Root, r.Name, r.Receiver, r.ReceiverType,
			r.Kind, r.NamespaceHint, r.Line, r.Arity); err != nil {
			return err
		}
	}
	return nil
}

// InsertAssets replaces the assets contributed by one file path.
func (t *Tx) InsertAssets(assets []model.Asset, line int) error {
	if len(assets) == 0 {
		return nil
	}
	paths := map[string]bool{}
	for _, a := range assets {
		if a.Path != "" {
			paths[a.Path] = true
		}
	}
	for p := range paths {
		var n int
		if err := t.queryRow(`SELECT COUNT(*) FROM assets WHERE path = ?`, p).Scan(&n); err != nil {
			return err
		}
		if n == 0 {
			continue
		}
		if _, err := t.delAssetEd.Exec(p, p); err != nil {
			return err
		}
		if _, err := t.delAssets.Exec(p); err != nil {
			return err
		}
	}
	stmt := t.insAsset
	for _, a := range assets {
		meta := "{}"
		if len(a.Meta) > 0 {
			if b, err := json.Marshal(a.Meta); err == nil {
				meta = string(b)
			}
		}
		if _, err := stmt.Exec(a.Key, a.Root, a.Kind, a.Path, a.Name, a.FileID, a.Line, meta); err != nil {
			return err
		}
	}
	return nil
}

// InsertAssetEdges appends asset edges.
func (t *Tx) InsertAssetEdges(edges []model.AssetEdge) error {
	if len(edges) == 0 {
		return nil
	}
	stmt := t.insAssetEdge
	for _, e := range edges {
		if _, err := stmt.Exec(e.Src, e.Dst, e.Kind, e.Line, e.Detail, e.Root); err != nil {
			return err
		}
	}
	return nil
}

// ClearGraph removes all symbol-level graph data (used before a full rebind).
func (s *Store) ClearGraph() error {
	for _, q := range []string{`DELETE FROM edges`, `UPDATE refs SET target=0, confidence=NULL`} {
		if _, err := s.db.Exec(q); err != nil {
			return err
		}
	}
	return nil
}

// InsertEdges bulk-inserts symbol edges in multi-row batches.
func (s *Store) InsertEdges(edges []model.ResolvedEdge) error {
	if len(edges) == 0 {
		return nil
	}
	tx, err := s.db.Begin()
	if err != nil {
		return err
	}
	const batch = 800
	for start := 0; start < len(edges); start += batch {
		end := start + batch
		if end > len(edges) {
			end = len(edges)
		}
		var b strings.Builder
		b.WriteString(`INSERT INTO edges(src, dst, kind, line, confidence) VALUES `)
		args := make([]any, 0, (end-start)*5)
		for i := start; i < end; i++ {
			if i > start {
				b.WriteByte(',')
			}
			b.WriteString(`(?,?,?,?,?)`)
			e := edges[i]
			args = append(args, e.Src, e.Dst, e.Kind, e.Line, e.Confidence)
		}
		if _, err := tx.Exec(b.String(), args...); err != nil {
			tx.Rollback()
			return err
		}
	}
	return tx.Commit()
}

// LoadRefs loads all references for binding.
type LoadedRef struct {
	ID            int64
	SrcID         int64
	FileID        int64
	Root          string
	Name          string
	ReceiverType  string
	Kind          string
	NamespaceHint string
	Line          int
	Arity         int
	Target        int64
}

// LoadRefs streams every reference row (kept for small projects / tests).
func (s *Store) LoadRefs() ([]LoadedRef, error) { return s.LoadRefsPage(0, 0) }

// LoadRefsPage returns at most limit refs with id > afterID, ordered by id.
// Paging keeps peak memory flat on million-reference projects.
func (s *Store) LoadRefsPage(afterID int64, limit int) ([]LoadedRef, error) {
	q := `SELECT id, src_id, file_id, COALESCE(root,''), name, COALESCE(receiver_type,''),
		kind, COALESCE(namespace_hint,''), COALESCE(line,0), COALESCE(arity,-1), COALESCE(target,0)
		FROM refs WHERE id > ? ORDER BY id`
	args := []any{afterID}
	if limit > 0 {
		q += " LIMIT ?"
		args = append(args, limit)
	}
	rows, err := s.db.Query(q, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	capHint := limit
	if capHint <= 0 || capHint > 65536 {
		capHint = 65536
	}
	out := make([]LoadedRef, 0, capHint)
	for rows.Next() {
		var r LoadedRef
		if err := rows.Scan(&r.ID, &r.SrcID, &r.FileID, &r.Root, &r.Name, &r.ReceiverType, &r.Kind,
			&r.NamespaceHint, &r.Line, &r.Arity, &r.Target); err != nil {
			return nil, err
		}
		out = append(out, r)
	}
	return out, rows.Err()
}

// ClearEdges drops the symbol edge table (refs keep their previous targets so
// the rebind can skip unchanged rows).
func (s *Store) ClearEdges() error {
	_, err := s.db.Exec(`DELETE FROM edges`)
	return err
}

// LoadSymbolRows loads the compact symbol table used by the binder.
type SymbolRow struct {
	ID         int64
	Root       string
	Lang       string
	Kind       string
	Name       string
	Qualname   string
	Namespace  string
	Container  string
	ParamCount int
}

// LoadSymbolRows streams all symbols for binding.
func (s *Store) LoadSymbolRows() ([]SymbolRow, error) {
	rows, err := s.db.Query(`SELECT id, root, lang, kind, name, qualname, COALESCE(namespace,''),
		COALESCE(container,''), COALESCE(param_count,-1) FROM symbols`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []SymbolRow
	for rows.Next() {
		var r SymbolRow
		if err := rows.Scan(&r.ID, &r.Root, &r.Lang, &r.Kind, &r.Name, &r.Qualname, &r.Namespace, &r.Container, &r.ParamCount); err != nil {
			return nil, err
		}
		out = append(out, r)
	}
	return out, rows.Err()
}

// RefTargetUpdate records the resolution of one reference.
type RefTargetUpdate struct {
	ID         int64
	Target     int64
	Confidence string
}

// UpdateRefTarget records reference resolutions in one bulk pass.
func (s *Store) UpdateRefTarget(updates []RefTargetUpdate) error {
	if len(updates) == 0 {
		return nil
	}
	tx, err := s.db.Begin()
	if err != nil {
		return err
	}
	if _, err := tx.Exec(`CREATE TEMP TABLE IF NOT EXISTS _ref_upd(id INTEGER PRIMARY KEY, target INTEGER, conf TEXT)`); err != nil {
		tx.Rollback()
		return err
	}
	if _, err := tx.Exec(`DELETE FROM _ref_upd`); err != nil {
		tx.Rollback()
		return err
	}
	const batch = 800
	for start := 0; start < len(updates); start += batch {
		end := start + batch
		if end > len(updates) {
			end = len(updates)
		}
		var b strings.Builder
		b.WriteString(`INSERT OR REPLACE INTO _ref_upd(id,target,conf) VALUES `)
		args := make([]any, 0, (end-start)*3)
		for i := start; i < end; i++ {
			if i > start {
				b.WriteByte(',')
			}
			b.WriteString(`(?,?,?)`)
			u := updates[i]
			args = append(args, u.ID, u.Target, u.Confidence)
		}
		if _, err := tx.Exec(b.String(), args...); err != nil {
			tx.Rollback()
			return err
		}
	}
	if _, err := tx.Exec(`UPDATE refs SET target = u.target, confidence = u.conf
		FROM _ref_upd u WHERE refs.id = u.id`); err != nil {
		tx.Rollback()
		return err
	}
	return tx.Commit()
}

// GuidRow is one guid -> asset path mapping (from the editor exporter).
type GuidRow struct {
	GUID string
	Path string
	Type string
}

// ReplaceGuidMap swaps the whole mapping table in one transaction.
func (s *Store) ReplaceGuidMap(rows []GuidRow) error {
	tx, err := s.db.Begin()
	if err != nil {
		return err
	}
	if _, err := tx.Exec(`DELETE FROM guid_map`); err != nil {
		tx.Rollback()
		return err
	}
	stmt, err := tx.Prepare(`INSERT OR REPLACE INTO guid_map(guid, path, name, type) VALUES(?,?,?,?)`)
	if err != nil {
		tx.Rollback()
		return err
	}
	defer stmt.Close()
	for i, r := range rows {
		if _, err := stmt.Exec(r.GUID, r.Path, filepath.Base(r.Path), r.Type); err != nil {
			tx.Rollback()
			return err
		}
		if i%5000 == 0 && i > 0 {
			// keep the transaction bounded on very large projects
			if err := tx.Commit(); err != nil {
				return err
			}
			if tx, err = s.db.Begin(); err != nil {
				return err
			}
			stmt.Close()
			if stmt, err = tx.Prepare(`INSERT OR REPLACE INTO guid_map(guid, path, name, type) VALUES(?,?,?,?)`); err != nil {
				return err
			}
		}
	}
	return tx.Commit()
}

// GuidCount returns how many guid mappings are loaded.
func (s *Store) GuidCount() int {
	var n int
	_ = s.db.QueryRow(`SELECT COUNT(*) FROM guid_map`).Scan(&n)
	return n
}

// ApplyBindings flushes reference targets and symbol edges in one transaction,
// halving commit/statement overhead during a rebind page.
func (s *Store) ApplyBindings(updates []RefTargetUpdate, edges []model.ResolvedEdge) error {
	if len(updates) == 0 && len(edges) == 0 {
		return nil
	}
	tx, err := s.db.Begin()
	if err != nil {
		return err
	}
	if len(updates) > 0 {
		if _, err := tx.Exec(`CREATE TEMP TABLE IF NOT EXISTS _ref_upd(id INTEGER PRIMARY KEY, target INTEGER, conf TEXT)`); err != nil {
			tx.Rollback()
			return err
		}
		if _, err := tx.Exec(`DELETE FROM _ref_upd`); err != nil {
			tx.Rollback()
			return err
		}
		const ub = 800
		for start := 0; start < len(updates); start += ub {
			end := start + ub
			if end > len(updates) {
				end = len(updates)
			}
			var b strings.Builder
			b.WriteString(`INSERT OR REPLACE INTO _ref_upd(id,target,conf) VALUES `)
			args := make([]any, 0, (end-start)*3)
			for i := start; i < end; i++ {
				if i > start {
					b.WriteByte(',')
				}
				b.WriteString(`(?,?,?)`)
				u := updates[i]
				args = append(args, u.ID, u.Target, u.Confidence)
			}
			if _, err := tx.Exec(b.String(), args...); err != nil {
				tx.Rollback()
				return err
			}
		}
		if _, err := tx.Exec(`UPDATE refs SET target = u.target, confidence = u.conf
			FROM _ref_upd u WHERE refs.id = u.id`); err != nil {
			tx.Rollback()
			return err
		}
	}
	const eb = 800
	for start := 0; start < len(edges); start += eb {
		end := start + eb
		if end > len(edges) {
			end = len(edges)
		}
		var b strings.Builder
		b.WriteString(`INSERT INTO edges(src, dst, kind, line, confidence) VALUES `)
		args := make([]any, 0, (end-start)*5)
		for i := start; i < end; i++ {
			if i > start {
				b.WriteByte(',')
			}
			b.WriteString(`(?,?,?,?,?)`)
			e := edges[i]
			args = append(args, e.Src, e.Dst, e.Kind, e.Line, e.Confidence)
		}
		if _, err := tx.Exec(b.String(), args...); err != nil {
			tx.Rollback()
			return err
		}
	}
	return tx.Commit()
}

// SetMeta writes a key/value pair.
func (s *Store) SetMeta(key, value string) error {
	_, err := s.db.Exec(`INSERT INTO meta(key,value) VALUES(?,?)
		ON CONFLICT(key) DO UPDATE SET value=excluded.value`, key, value)
	return err
}

// GetMeta reads a key/value pair.
func (s *Store) GetMeta(key string) string {
	var v string
	if err := s.db.QueryRow(`SELECT value FROM meta WHERE key=?`, key).Scan(&v); err != nil {
		return ""
	}
	return v
}

// Counts returns table counts.
func (s *Store) Counts() (files, symbols, refs, edges, assets, assetEdges int) {
	row := s.db.QueryRow(`SELECT
		(SELECT COUNT(*) FROM files), (SELECT COUNT(*) FROM symbols), (SELECT COUNT(*) FROM refs),
		(SELECT COUNT(*) FROM edges), (SELECT COUNT(*) FROM assets), (SELECT COUNT(*) FROM asset_edges)`)
	_ = row.Scan(&files, &symbols, &refs, &edges, &assets, &assetEdges)
	return
}

// PruneFiles removes DB rows whose (root, path) is absent from the scan set.
func (s *Store) PruneFiles(present map[string]bool) (int, error) {
	states, err := s.LoadFileStates()
	if err != nil {
		return 0, err
	}
	var stale []struct {
		id   int64
		root string
		path string
	}
	for k, st := range states {
		root, path, _ := strings.Cut(k, "|")
		if !present[k] {
			stale = append(stale, struct {
				id   int64
				root string
				path string
			}{st.ID, root, path})
		}
	}
	if len(stale) == 0 {
		return 0, nil
	}
	tx, err := s.Begin()
	if err != nil {
		return 0, err
	}
	for _, f := range stale {
		if err := tx.DeleteFile(f.id, f.path); err != nil {
			tx.Rollback()
			return 0, err
		}
	}
	if err := tx.Commit(); err != nil {
		return 0, err
	}
	return len(stale), nil
}

// NowUnix is a helper for meta timestamps.
func NowUnix() int64 { return time.Now().Unix() }
