package indexer

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/neko233-com/codegraph233/internal/config"
	"github.com/neko233-com/codegraph233/internal/query"
)

const (
	csSample = `namespace Game {
  public class Player {
    private int hp;
    public void Hit(int d) { Damage(d); }
    public void Damage(int d) { hp -= d; }
  }
}`
	goSample = `package battle

type Unit struct{ HP int }

func (u *Unit) Hit(d int) int { return u.damage(d) }

func (u *Unit) damage(d int) int { u.HP -= d; return u.HP }
`
	rsSample = `pub struct Player { hp: i32 }

impl Player {
    pub fn hit(&mut self, d: i32) -> i32 {
        self.damage(d);
        self.hp
    }
    pub fn damage(&mut self, d: i32) { self.hp -= d; }
}
`
	tsSample = `export class Player {
  hp = 0;
  hit(d: number): number { this.damage(d); return this.hp; }
  damage(d: number): void { this.hp -= d; }
}
`
)

func writeFixture(t *testing.T, dir, rel, content string) {
	t.Helper()
	path := filepath.Join(dir, filepath.FromSlash(rel))
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

// TestIndexMultiLanguage exercises the full pipeline (scan -> parse -> bind ->
// query) for every implemented front-end.
func TestIndexMultiLanguage(t *testing.T) {
	dir := t.TempDir()
	writeFixture(t, dir, "src/Player.cs", csSample)
	writeFixture(t, dir, "src/battle.go", goSample)
	writeFixture(t, dir, "src/player.rs", rsSample)
	writeFixture(t, dir, "src/player.ts", tsSample)
	writeFixture(t, dir, "src/Config.xlsx.placeholder", "")

	cfg := &config.Config{
		Version:   1,
		Strategy:  "auto",
		Dir:       dir,
		Languages: []string{"csharp", "go", "rust", "typescript"},
		Roots:     []config.Root{{Key: "client", Path: ".", Role: "frontend"}},
	}
	stats, err := Run(cfg, Options{Force: true, Quiet: true})
	if err != nil {
		t.Fatalf("index: %v", err)
	}
	for _, lang := range []string{"csharp", "go", "rust", "typescript"} {
		if stats.ByLang[lang] == 0 {
			t.Errorf("lang %q not indexed; byLang=%v", lang, stats.ByLang)
		}
	}
	if stats.Symbols == 0 || stats.Edges == 0 {
		t.Fatalf("empty graph: symbols=%d edges=%d", stats.Symbols, stats.Edges)
	}

	eng, err := query.Open(cfg)
	if err != nil {
		t.Fatal(err)
	}
	defer eng.Close()

	hits, err := eng.SearchSymbols("Player", nil, "", "", "", 20)
	if err != nil {
		t.Fatal(err)
	}
	if len(hits) < 3 {
		t.Fatalf("expected Player in csharp/rust/typescript, got %d", len(hits))
	}
	// each language's Player should expose hit -> damage as a call edge
	members, err := eng.SearchSymbols("hit", []string{"method"}, "", "", "", 20)
	if err != nil {
		t.Fatal(err)
	}
	if len(members) == 0 {
		t.Fatalf("no hit methods indexed")
	}
	sawDamage := 0
	for _, m := range members {
		callees, err := eng.Related(m.ID, "out", []string{"calls"}, 1, 50)
		if err != nil {
			t.Fatal(err)
		}
		for _, c := range callees {
			if c.Name == "damage" || c.Name == "Damage" {
				sawDamage++
			}
		}
	}
	if sawDamage < 3 {
		t.Errorf("expected hit->damage in csharp/go/rust/typescript, saw %d", sawDamage)
	}

	// language policy: a config with only csharp must not index the others
	cfg2 := &config.Config{
		Version: 1, Strategy: "auto", Dir: dir,
		Languages: []string{"csharp"},
		Roots:     []config.Root{{Key: "client", Path: ".", Role: "frontend"}},
	}
	if !cfg2.LanguageEnabled("csharp") || cfg2.LanguageEnabled("go") || cfg2.LanguageEnabled("rust") || cfg2.LanguageEnabled("typescript") {
		t.Errorf("policy filter broken")
	}
	if (&config.Config{Dir: dir}).LanguageEnabled("lua") {
		t.Errorf("lua must stay planned/disabled")
	}
}
