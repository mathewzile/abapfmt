# Benchmarks

`cargo run --release -p abapfmt_formatter --example bench [sample]` formats replicated sample files
with all rules, the default `lineWidth` and the token check. Identifiers are made unique per copy,
so that the statement cache of the parser gets no help from repeated code. `--example prof`
shows the time per rule.

Windows 11, release build, single thread. `cleanup-sample.abap` (default configuration):

| Lines | Time | Lines/s |
|---|---|---|
| 161 | 11 ms | 14k |
| 1,601 | 52 ms | 31k |
| 8,001 | 261 ms | 31k |
| 32,001 | 1.2 s | 27k |

`cleanup-sample.acds` reaches the same throughput (12,600 lines in 0.41 s). For comparison, the
TypeScript port measured 229 ms for 1.6k lines and 7.6 s for 32k lines (synthetic input, same
sample, `NEXT-STEPS.md` of that repository).

What made the difference, in order of effect:

1. The statement matcher (a port of abaplint's) is the most expensive part of a parse, and rules
   change only whitespace and letter case. Results are cached by the exact tokens of a statement.
2. `format` parses once and keeps the parsed document for as long as rules leave it unchanged,
   instead of once per rule. DDL rules no longer parse ABAP sources.
3. Two scans over all commands in `abapfmt_syntax::parse` were quadratic; the 8k-line parse went
   from 55 ms to 9 ms.

Remaining cost is about one parse per rule that changed the text (13 of 46 on the sample), plus
the final token check and the reflow. Rules that merge their changes into a shared document would
remove most of that; at about 30k lines/s this is not needed yet.
