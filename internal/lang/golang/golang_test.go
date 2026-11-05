package golang

import "testing"

const sample = `package battle

import (
	s "g.com/x/svc"
	"fmt"
)

type Unit struct {
	HP   int
	svc  s.Client
	Name string
}

type Hitter interface {
	Hit(d int) int
}

func (u *Unit) Hit(d int) int {
	s.Calc(d)
	u.HP++
	fmt.Println(u.Name)
	return d
}

func NewUnit() *Unit {
	return &Unit{}
}

func Run() {
	u := NewUnit()
	u.Hit(1)
}
`

func TestParseGoStructure(t *testing.T) {
	res := Parse("internal/battle/unit.go", "server", "g.com/x/game", "internal/battle", []byte(sample))
	if len(res.Errors) > 0 {
		t.Fatalf("errors: %v", res.Errors)
	}
	names := map[string]string{}
	for _, s := range res.Symbols {
		names[s.Name] = s.Kind
	}
	for _, want := range []string{"Unit", "Hitter", "Hit", "NewUnit", "Run", "HP", "Name", "svc"} {
		if _, ok := names[want]; !ok {
			t.Errorf("missing symbol %q", want)
		}
	}
	if names["Unit"] != "struct" {
		t.Errorf("Unit kind = %q", names["Unit"])
	}
	if names["Hitter"] != "interface" {
		t.Errorf("Hitter kind = %q", names["Hitter"])
	}
	for _, s := range res.Symbols {
		if s.Name == "Hit" && s.Qualname == "g.com/x/game/internal/battle.Unit.Hit" {
			if s.Kind != "method" {
				t.Errorf("Unit.Hit kind = %q", s.Kind)
			}
		}
	}
}

func TestParseGoRefs(t *testing.T) {
	res := Parse("internal/battle/unit.go", "server", "g.com/x/game", "internal/battle", []byte(sample))
	has := func(kind, name string) bool {
		for _, r := range res.Refs {
			if r.Kind == kind && r.Name == name {
				return true
			}
		}
		return false
	}
	for _, w := range []struct{ kind, name string }{
		{"calls", "Calc"},
		{"calls", "Println"},
		{"calls", "NewUnit"},
		{"calls", "Hit"},
		{"references", "HP"},
		{"references", "Name"},
		{"constructs", "Unit"},
	} {
		if !has(w.kind, w.name) {
			t.Errorf("missing ref %s %q", w.kind, w.name)
		}
	}
	for _, r := range res.Refs {
		if r.Name == "Calc" && r.ReceiverType != "g.com/x/svc" {
			t.Errorf("Calc receiver type = %q, want import path", r.ReceiverType)
		}
	}
}
