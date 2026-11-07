# Ordinary JVM bytecode and facts

0.4.1-dev directly reads ordinary unencrypted `.class` and `.jar` inputs using a
bounded native Rust reader, without Java installation or execution. Enable
`jvm`, `java` (also enables bytecode), or `auto`; add the containing directory as
a root. Keep third-party artifacts and generated indexes ignored. Unknown/private
containers fail; there is no private decoding, activation bypass or VM patch.

The reader handles standard constant pool tags, modified UTF-8, two-slot numeric
constants, instruction widths including wide/table/lookup switch, declared
members, inheritance and static calls/fields/bootstrap facts. Unknown attributes
are skipped by bounded length. It is not a complete JVM verifier or decompiler.
Every non-directory JAR member receives SHA-256/size as a `jvm-resource` asset
without extraction. Class identity retains entry, major version and SHA. Binary
symbol/reference lines are zero; `include_source` reports source unavailable.

Multi-release selection honors manifest `Multi-Release: true` and `jvm_release`
(default 8), choosing the highest class variant <= target. All resources remain
inventoried. Bounds: 512 MiB input, 16 MiB/class, 256 MiB expanded archive and
30,000 members. Invalid selected classes fail the entire artifact atomically.

Optional independent ASM exporter: `tools/jvm/BytecodeRelations.java`. Provide an
existing ASM 9 classpath; no automatic download. Compile with `javac -cp <asm>
-d <classes> tools/jvm/BytecodeRelations.java`; run `java -cp <classes-and-asm>
BytecodeRelations <ordinary.jar> <new-output.cg-jvm.jsonl>`. Use platform classpath
separators and hidden, bounded, quiet Windows processes. This exporter does not
select MR duplicates and explicitly rejects ConstantDynamic LDC inputs instead
of silently dropping them; use the native reader for these cases. Owned compile-only fixture,
source and compiler/hash provenance live in `testdata/jvm/`.

Only `*.cg-jvm.jsonl` imports facts. Version 1 starts with `kind=manifest`,
`format=codegraph233.jvm-facts`, `version=1`, artifact SHA-256. Class ownership and
descriptors must agree. Final summary must match class/method/field/direct/dynamic
counts with zero failed classes. Partial, duplicate, inconsistent or unknown rows
are rejected. Import bounds: 64 MiB, 1 MiB/record, one million records. A digest
identifies data but does not authenticate an arbitrary producer. Fact line
numbers refer to JSONL records, not recovered Java source.

Method and field identities include exact owner/name/descriptors. Binding never
substitutes name/arity guesses. Missing external declarations and ambiguous
duplicates stay unresolved. Virtual/interface calls identify declared bytecode
targets, not all runtime implementations. Dynamic sites stay `dynamic-call`;
bootstrap handles are separate `bootstrap` relationships, not runtime target
proof. Erased source/comments, reflection targets, native bodies and private
resources cannot be reconstructed by this graph.
