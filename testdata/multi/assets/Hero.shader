Shader "Game/HeroLit" {
    Properties {
        _MainTex ("Main Texture", 2D) = "white" {}
        _Tint ("Tint", Color) = (1,1,1,1)
        _Gloss ("Gloss", Range(0,1)) = 0.5
    }
    SubShader {
        Tags { "RenderType"="Opaque" }
        Pass {
            CGPROGRAM
            #pragma vertex vert
            #pragma fragment frag
            #pragma multi_compile _ QUALITY_LOW QUALITY_HIGH
            #include "UnityCG.cginc"

            float4 _Tint;
            sampler2D _MainTex;

            float4 tinted(float4 c) {
                return c * _Tint;
            }

            void vert(inout float4 pos) {
                pos = UnityObjectToClipPos(pos);
            }

            fixed4 frag() : SV_Target {
                return tinted(tex2D(_MainTex, float2(0,0)));
            }
            ENDCG
        }
    }
}
