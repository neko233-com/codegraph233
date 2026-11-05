package rust

import (
	"testing"

	"github.com/neko233-com/codegraph233/internal/model"
)

const sample = `
use crate::combat::{Unit, Skill};

pub struct Player {
    hp: i32,
    unit: Unit,
}

pub trait Hitter {
    fn hit(&self, target: &Unit) -> i32;
}

impl Hitter for Player {
    fn hit(&self, target: &Unit) -> i32 {
        let dmg = Skill::calc(self.hp);
        target.apply(dmg);
        dmg
    }
}

pub enum Mode { Idle, Run }

pub fn main() {}
`

func TestRustStructure(t *testing.T) {
	res := Parse("src/game.rs", "client", []byte(sample))
	find := func(kind, name string) *model.Symbol {
		for i := range res.Symbols {
			if res.Symbols[i].Kind == kind && res.Symbols[i].Name == name {
				return &res.Symbols[i]
			}
		}
		return nil
	}
	for _, w := range []struct{ kind, name string }{
		{model.KindStruct, "Player"},
		{model.KindInterface, "Hitter"},
		{model.KindEnum, "Mode"},
		{model.KindEnumMember, "Idle"},
		{model.KindField, "hp"},
		{model.KindField, "unit"},
		{model.KindMethod, "hit"},
		{model.KindFunction, "main"},
	} {
		if find(w.kind, w.name) == nil {
			t.Errorf("missing %s %q; got %v", w.kind, w.name, names(res.Symbols))
		}
	}
	if s := find(model.KindStruct, "Player"); s != nil && s.Qualname != "crate::game::Player" {
		t.Errorf("Player qualname = %q", s.Qualname)
	}
	found := false
	for _, sym := range res.Symbols {
		if sym.Kind == model.KindMethod && sym.Name == "hit" && sym.Container == "crate::game::Player" {
			found = true
		}
	}
	if !found {
		t.Errorf("missing impl method hit on Player; got %v", names(res.Symbols))
	}
}

func TestRustRefs(t *testing.T) {
	res := Parse("src/game.rs", "client", []byte(sample))
	has := func(kind, name string) *model.Ref {
		for i := range res.Refs {
			if res.Refs[i].Kind == kind && res.Refs[i].Name == name {
				return &res.Refs[i]
			}
		}
		return nil
	}
	for _, w := range []struct{ kind, name string }{
		{"implements", "Hitter"},
		{"calls", "calc"},
		{"calls", "apply"},
		{"references", "Unit"},
	} {
		if has(w.kind, w.name) == nil {
			t.Errorf("missing ref %s %q; got %v", w.kind, w.name, refNames(res.Refs))
		}
	}
	if r := has("calls", "calc"); r != nil && r.ReceiverType != "crate::combat::Skill" {
		t.Errorf("calc receiver type = %q, want crate::combat::Skill", r.ReceiverType)
	}
}

func names(syms []model.Symbol) []string {
	out := make([]string, 0, len(syms))
	for _, s := range syms {
		out = append(out, s.Kind+":"+s.Name)
	}
	return out
}

func refNames(refs []model.Ref) []string {
	out := make([]string, 0, len(refs))
	for _, r := range refs {
		out = append(out, r.Kind+":"+r.Name)
	}
	return out
}
