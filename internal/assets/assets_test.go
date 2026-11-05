package assets

import (
	"os"
	"path/filepath"
	"testing"
)

const prefabSample = `%YAML 1.1
%TAG !u! tag:unity3d.com,2011:
--- !u!1 &100
GameObject:
  m_ObjectHideFlags: 0
  m_Component:
  - component: {fileID: 101}
  m_Name: Root
--- !u!4 &101
Transform:
  m_GameObject: {fileID: 100}
  m_Father: {fileID: 0}
--- !u!1 &200
GameObject:
  m_Component:
  - component: {fileID: 201}
  m_Name: Child
--- !u!4 &201
Transform:
  m_GameObject: {fileID: 200}
  m_Father: {fileID: 101}
--- !u!114 &300
MonoBehaviour:
  m_GameObject: {fileID: 200}
  m_Script: {fileID: 11500000, guid: 0123456789abcdef0123456789abcdef, type: 3}
--- !u!1001 &400
PrefabInstance:
  m_SourcePrefab: {fileID: 100100000, guid: fedcba9876543210fedcba9876543210, type: 3}
`

func TestParsePrefab(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "Sample.prefab")
	if err := os.WriteFile(path, []byte(prefabSample), 0o644); err != nil {
		t.Fatal(err)
	}
	res := ParsePrefab(path, "Assets/Sample.prefab", "client")
	if len(res.Errors) > 0 {
		t.Fatalf("errors: %v", res.Errors)
	}
	names := map[string]bool{}
	for _, a := range res.Assets {
		names[a.Name] = true
	}
	if !names["Root"] || !names["Child"] {
		t.Fatalf("missing gameobjects, got %v", names)
	}
	var hasChild, hasScript, hasInstantiate bool
	for _, e := range res.AssetEdges {
		switch e.Kind {
		case "contains":
			if e.Detail == "child" {
				hasChild = true
			}
		case "uses_script":
			hasScript = true
		case "instantiates":
			hasInstantiate = true
		}
	}
	if !hasChild {
		t.Error("missing parent->child contains edge")
	}
	if !hasScript {
		t.Error("missing m_Script uses_script edge")
	}
	if !hasInstantiate {
		t.Error("missing PrefabInstance instantiates edge")
	}
}

func TestParseShader(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "Test.shader")
	src := `Shader "Game/VfxTest" {
  Properties {
    _MainTex ("Texture", 2D) = "white" {}
    _Color ("Tint", Color) = (1,1,1,1)
  }
  SubShader {
    Pass {
      Name "Forward"
      CGPROGRAM
      #pragma multi_compile _ USE_ALPHA
      #include "UnityCG.cginc"
      half4 frag(v2f i) : SV_Target { return 1; }
      ENDCG
    }
  }
}`
	if err := os.WriteFile(path, []byte(src), 0o644); err != nil {
		t.Fatal(err)
	}
	res, err := ParseShader(path, "Assets/Test.shader", "client")
	if err != nil {
		t.Fatal(err)
	}
	var props, passes, includes int
	for _, a := range res.Assets {
		switch a.Kind {
		case "shader_property":
			props++
		case "shader_pass":
			passes++
		}
	}
	for _, e := range res.AssetEdges {
		if e.Kind == "includes" {
			includes++
		}
	}
	if props != 2 {
		t.Errorf("properties = %d, want 2", props)
	}
	if passes < 1 {
		t.Errorf("passes = %d, want >=1", passes)
	}
	if includes != 1 {
		t.Errorf("includes = %d, want 1", includes)
	}
	if len(res.Assets) == 0 || res.Assets[0].Name != "Game/VfxTest" {
		t.Errorf("shader name = %v", res.Assets)
	}
}
