package csharp

import (
	"testing"

	"github.com/neko233-com/game-project-graph/internal/model"
)

const sample = `using System;
namespace Game.Core {
  public interface IThing { void Do(int x); }
  public class Base { protected int Hp; }
  public partial class Player : Base, IThing {
    private int hp;
    public int MaxHp { get; set; }
    public event System.Action Died;
    public Player(int h) { hp = h; }
    [SerializeField] private Buff buff;
    public void Do(int x) {
      Service.Run(x, 2);
      Helper.Calc(hp);
      var b = new Buff();
      b.Apply();
      this.MaxHp = Helper.Calc(b.Value);
    }
    public int Level => hp / 2;
  }
  public enum Mode { Idle = 0, Run }
}
`

func TestParseStructure(t *testing.T) {
	res := Parse("Assets/Scripts/Player.cs", "client", []byte(sample))
	find := func(kind, name string) *model.Symbol {
		for i := range res.Symbols {
			if res.Symbols[i].Kind == kind && res.Symbols[i].Name == name {
				return &res.Symbols[i]
			}
		}
		return nil
	}
	want := []struct{ kind, name string }{
		{model.KindInterface, "IThing"},
		{model.KindClass, "Base"},
		{model.KindClass, "Player"},
		{model.KindEnum, "Mode"},
		{model.KindEnumMember, "Idle"},
		{model.KindMethod, "Do"},
		{model.KindProperty, "MaxHp"},
		{model.KindEvent, "Died"},
		{model.KindConstructor, "Player"},
		{model.KindField, "hp"},
		{model.KindField, "buff"},
		{model.KindProperty, "Level"},
	}
	for _, w := range want {
		if find(w.kind, w.name) == nil {
			t.Errorf("missing %s %q", w.kind, w.name)
		}
	}
	if p := find(model.KindClass, "Player"); p != nil {
		if p.Qualname != "Game.Core.Player" {
			t.Errorf("qualname = %q, want Game.Core.Player", p.Qualname)
		}
		if p.EndLine <= p.StartLine {
			t.Errorf("end line %d <= start %d", p.EndLine, p.StartLine)
		}
	}
	if d := find(model.KindMethod, "Do"); d != nil && d.ParamCount != 1 {
		t.Errorf("Do param count = %d, want 1", d.ParamCount)
	}
}

func TestParseRefs(t *testing.T) {
	res := Parse("Assets/Scripts/Player.cs", "client", []byte(sample))
	has := func(kind, name string) *model.Ref {
		for i := range res.Refs {
			if res.Refs[i].Kind == kind && res.Refs[i].Name == name {
				return &res.Refs[i]
			}
		}
		return nil
	}
	for _, w := range []struct{ kind, name string }{
		{"inherits", "Base"},
		{"inherits", "IThing"},
		{"calls", "Run"},
		{"calls", "Calc"},
		{"calls", "Apply"},
		{"constructs", "Buff"},
	} {
		if has(w.kind, w.name) == nil {
			t.Errorf("missing ref %s %q; refs=%v", w.kind, w.name, refPairs(res.Refs))
		}
	}
	if r := has("calls", "Apply"); r != nil && r.ReceiverType != "Buff" {
		t.Errorf("Apply receiver type = %q, want Buff", r.ReceiverType)
	}
	if r := has("calls", "Run"); r != nil && r.Receiver != "Service" {
		t.Errorf("Run receiver = %q, want Service", r.Receiver)
	}
	if r := has("calls", "Calc"); r != nil && r.Arity != 1 {
		t.Errorf("Calc arity = %d, want 1", r.Arity)
	}
}

func TestParseSkipsCommentsAndStrings(t *testing.T) {
	src := `class A {
  void M() {
    // Fake.Call();
    var s = "Fake.Call()";
    /* Fake.Call(); */
    Real.Go();
  }
}`
	res := Parse("A.cs", "client", []byte(src))
	for _, r := range res.Refs {
		if r.Name == "Call" {
			t.Fatalf("comment/string produced a ref: %+v", r)
		}
	}
	found := false
	for _, r := range res.Refs {
		if r.Name == "Go" {
			found = true
		}
	}
	if !found {
		t.Fatal("missing Real.Go call")
	}
}

func TestFieldInitializersAreNotMethods(t *testing.T) {
	src := `class A {
  private readonly Random _random = new Random();
  public Stage Stage { get; private set; } = Stage.None;
  private int _count = 0;
  public void Run() { }
}`
	res := Parse("A.cs", "client", []byte(src))
	byName := map[string]model.Symbol{}
	for _, s := range res.Symbols {
		byName[s.Name] = s
	}
	if s, ok := byName["_random"]; !ok || s.Kind != model.KindField {
		t.Errorf("_random = %+v, want field", s)
	}
	if s, ok := byName["_count"]; !ok || s.Kind != model.KindField {
		t.Errorf("_count = %+v, want field", s)
	}
	if s, ok := byName["Stage"]; !ok || s.Kind != model.KindProperty {
		t.Errorf("Stage = %+v, want property", s)
	}
	for _, bad := range []string{"None", "get", "set", "Random", "new"} {
		if s, ok := byName[bad]; ok {
			t.Errorf("spurious symbol %q: %+v", bad, s)
		}
	}
}

func refPairs(refs []model.Ref) []string {
	out := make([]string, 0, len(refs))
	for _, r := range refs {
		out = append(out, r.Kind+":"+r.Name)
	}
	return out
}
