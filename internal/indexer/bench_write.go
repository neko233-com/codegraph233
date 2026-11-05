package indexer

import (
	"database/sql"
	"fmt"
	"os"
	"path/filepath"
	"time"

	_ "modernc.org/sqlite"
)

// BenchWriteResult is the write-path benchmark (symbols/refs/edges inserts).
type BenchWriteResult struct {
	Refs     int   `json:"refs"`
	Edges    int   `json:"edges"`
	InsertMS int64 `json:"insert_ms"`
	TotalMS  int64 `json:"total_ms"`
	DbBytes  int64 `json:"db_bytes"`
}

// BenchWrite inserts synthetic rows through the same schema the indexer uses.
// It isolates the storage engine cost (modernc SQLite in Go) from parsing.
func BenchWrite(dir string, refs, edges int) (*BenchWriteResult, error) {
	if refs <= 0 {
		refs = 1000000
	}
	if edges <= 0 {
		edges = 800000
	}
	dbPath := filepath.Join(dir, "bench-write.db")
	_ = os.Remove(dbPath)
	_ = os.Remove(dbPath + "-wal")
	_ = os.Remove(dbPath + "-shm")

	start := time.Now()
	dsn := "file:" + filepath.ToSlash(dbPath) +
		"?_pragma=journal_mode(OFF)&_pragma=synchronous(OFF)&_pragma=busy_timeout(8000)" +
		"&_pragma=temp_store(MEMORY)&_pragma=cache_size(-131072)&_pragma=mmap_size(268435456)"
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	defer db.Close()
	db.SetMaxOpenConns(1)

	schema := `
CREATE TABLE refs (
  id INTEGER PRIMARY KEY, src_id INTEGER, src_key TEXT, file_id INTEGER, root TEXT,
  name TEXT, receiver TEXT, receiver_type TEXT, kind TEXT, namespace_hint TEXT,
  line INTEGER, arity INTEGER, target INTEGER, confidence TEXT);
CREATE TABLE edges (src INTEGER, dst INTEGER, kind TEXT, line INTEGER, confidence TEXT);
CREATE INDEX idx_edges_src ON edges(src);
CREATE INDEX idx_edges_dst ON edges(dst);
CREATE INDEX idx_refs_target ON refs(target);
`
	if _, err := db.Exec(schema); err != nil {
		return nil, err
	}

	insertStart := time.Now()
	if err := writeBenchRows(db, refs, edges); err != nil {
		return nil, err
	}
	insertMS := time.Since(insertStart).Milliseconds()

	st, _ := os.Stat(dbPath)
	res := &BenchWriteResult{
		Refs: refs, Edges: edges, InsertMS: insertMS,
		TotalMS: time.Since(start).Milliseconds(),
	}
	if st != nil {
		res.DbBytes = st.Size()
	}
	_ = db.Close()
	return res, nil
}

func writeBenchRows(db *sql.DB, refs, edges int) error {
	tx, err := db.Begin()
	if err != nil {
		return err
	}
	refStmt, err := tx.Prepare(`INSERT INTO refs
		(src_id, src_key, file_id, root, name, receiver, receiver_type, kind, namespace_hint, line, arity, target, confidence)
		VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)`)
	if err != nil {
		tx.Rollback()
		return err
	}
	defer refStmt.Close()
	for i := 0; i < refs; i++ {
		if _, err := refStmt.Exec(i%50000, "k", i%48000, "client", "Name", "", "", "calls", "NS", i%900, 1, 0, ""); err != nil {
			tx.Rollback()
			return fmt.Errorf("ref %d: %w", i, err)
		}
	}
	edgeStmt, err := tx.Prepare(`INSERT INTO edges(src, dst, kind, line, confidence) VALUES(?,?,?,?,?)`)
	if err != nil {
		tx.Rollback()
		return err
	}
	defer edgeStmt.Close()
	for i := 0; i < edges; i++ {
		if _, err := edgeStmt.Exec(i%50000, i%40000, "calls", i%900, "exact"); err != nil {
			tx.Rollback()
			return fmt.Errorf("edge %d: %w", i, err)
		}
	}
	return tx.Commit()
}
