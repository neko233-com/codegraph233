package typescript

import (
	"testing"

	"github.com/neko233-com/codegraph233/internal/model"
)

const sample = `
import { Unit } from "./unit";
import { Skill as S } from "./skill";

export interface Hitter {
  hit(t: Unit): number;
}

export class Player implements Hitter {
  hp: number;
  unit: Unit;
  constructor(hp: number) { this.hp = hp; }
  hit(t: Unit): number {
    const dmg = S.calc(this.hp);
    t.apply(dmg);
    return dmg;
  }
}

export function main(): void {
  const p = new Player(1);
  p.hit(null as any);
}

export const helper = (x: number) => main();
`

func TestTypeScriptStructure(t *testing.T) {
	res := Parse("src/game.ts", "web", []byte(sample))
	find := func(kind, name string) *model.Symbol {
		for i := range res.Symbols {
			if res.Symbols[i].Kind == kind && res.Symbols[i].Name == name {
				return &res.Symbols[i]
			}
		}
		return nil
	}
	for _, w := range []struct{ kind, name string }{
		{model.KindInterface, "Hitter"},
		{model.KindClass, "Player"},
		{model.KindConstructor, "constructor"},
		{model.KindMethod, "hit"},
		{model.KindFunction, "main"},
		{model.KindFunction, "helper"},
		{model.KindProperty, "hp"},
		{model.KindProperty, "unit"},
	} {
		if find(w.kind, w.name) == nil {
			t.Errorf("missing %s %q; got %v", w.kind, w.name, symbolNames(res.Symbols))
		}
	}
	if c := find(model.KindClass, "Player"); c != nil && c.Qualname != "src.game.Player" {
		t.Errorf("Player qualname = %q", c.Qualname)
	}
}

func TestTypeScriptRefs(t *testing.T) {
	res := Parse("src/game.ts", "web", []byte(sample))
	has := func(kind, name string) *model.Ref {
		for i := range res.Refs {
			if res.Refs[i].Kind == kind && res.Refs[i].Name == name {
				return &res.Refs[i]
			}
		}
		return nil
	}
	if has("implements", "Hitter") == nil {
		t.Errorf("missing implements Hitter; got %v", refNames(res.Refs))
	}
	if has("constructs", "Player") == nil {
		t.Errorf("missing constructs Player")
	}
	if r := has("calls", "calc"); r == nil {
		t.Errorf("missing calls calc")
	} else if r.ReceiverType != "src.skill.Skill" {
		t.Errorf("calc receiver type = %q, want src.skill.Skill", r.ReceiverType)
	}
	if has("calls", "hit") == nil {
		t.Errorf("missing calls hit")
	}
	if has("calls", "apply") == nil {
		t.Errorf("missing calls apply")
	}
}

func symbolNames(syms []model.Symbol) []string {
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
