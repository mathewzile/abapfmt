// Turn abaplint's statement/expression grammar into a JSON IR.
//
// Usage: node tools/gen-grammar/gen.cjs <abaplint-core-package-dir> <out-dir>
//
// abaplint (MIT, https://github.com/abaplint/abaplint) defines its grammar as combinator objects
// (seq/alt/opt/...). Instead of parsing the TypeScript source we load the compiled package and
// walk those objects, which keeps the generator independent of how a grammar file is written.
// Anything that is not a known combinator is emitted as a `custom` node and counted as needing a
// hand-written override.
"use strict";
const fs = require("fs");
const path = require("path");

const [pkgDir, outDir] = process.argv.slice(2);
if (!pkgDir || !outDir) {
  console.error("usage: gen.cjs <abaplint-core-package-dir> <out-dir>");
  process.exit(2);
}
const src = path.resolve(pkgDir, "build/src");
const { Expression } = require(path.join(src, "abap/2_statements/combi.js"));
const statements = require(path.join(src, "abap/2_statements/statements"));
const expressions = require(path.join(src, "abap/2_statements/expressions"));
const version = require(path.join(src, "version.js"));
const pkgVersion = require(path.resolve(pkgDir, "package.json")).version;

// ---- node arena with structural interning ----
const nodes = [];
const interned = new Map();
function mk(node) {
  const key = JSON.stringify(node);
  let id = interned.get(key);
  if (id === undefined) {
    id = nodes.length;
    nodes.push(node);
    interned.set(key, id);
  }
  return id;
}

const exprIds = new Map(); // expression class name -> index
const exprQueue = [];
function exprRef(inst) {
  const name = inst.constructor.name;
  if (!exprIds.has(name)) {
    exprIds.set(name, exprIds.size);
    exprQueue.push([name, inst]);
  }
  return exprIds.get(name);
}

const customKinds = new Map(); // class name -> count
const tokenNames = new Set();
const regexes = new Set();

function walk(o) {
  if (o instanceof Expression) return mk({ k: "expr", e: exprRef(o) });
  const cls = o.constructor.name;
  switch (cls) {
    case "Word":
      return mk({ k: "word", s: o.s });
    case "WordSequence":
      return mk({ k: "seq", c: o.words.map(walk) });
    case "Regex":
      regexes.add(o.regexp.source + "/" + o.regexp.flags);
      return mk({ k: "regex", re: o.regexp.source, flags: o.regexp.flags });
    case "Token":
      tokenNames.add(o.name);
      return mk({ k: "tok", name: o.name });
    case "Sequence":
      return mk({ k: "seq", c: o.list.map(walk) });
    case "Alternative":
      return mk({ k: "alt", c: o.list.map(walk) });
    case "AlternativePriority":
      return mk({ k: "altPrio", c: o.list.map(walk) });
    case "Permutation":
      return mk({ k: "per", c: o.list.map(walk) });
    case "Optional":
      return mk({ k: "opt", c: walk(o.optional) });
    case "OptionalPriority":
      return mk({ k: "optPrio", c: walk(o.optional) });
    case "Star":
      return mk({ k: "star", c: walk(o.sta) });
    case "StarPriority":
      return mk({ k: "starPrio", c: walk(o.sta) });
    case "Plus":
      return mk({ k: "plus", c: walk(o.plu) });
    case "PlusPriority":
      return mk({ k: "plusPrio", c: walk(o.plu) });
    case "Vers":
      return mk({ k: "ver", rel: o.release.ordinal, openAbap: o.also === "OpenABAP", c: walk(o.runnable) });
    case "LangVers":
      return mk({ k: "verLang", lang: o.langVersion, c: walk(o.runnable) });
    case "LangVersNot":
      return mk({ k: "verNotLang", lang: o.langVersion, c: walk(o.runnable) });
    case "FailCombinator":
      return mk({ k: "fail" });
    case "FailStar":
      return mk({ k: "failStar" });
    case "StopBefore1":
      return mk({ k: "stopBefore1", w: o.words });
    case "StopBefore2":
      return mk({ k: "stopBefore2", w: [o.t1, o.t2] });
    default:
      customKinds.set(cls, (customKinds.get(cls) ?? 0) + 1);
      return mk({ k: "custom", cls });
  }
}

// ---- walk every statement and expression ----
const NON_GRAMMAR = new Set(["Unknown", "Comment", "Empty", "MacroCall", "MacroContent", "MacroRecursion", "NativeSQL"]);
const stmtOut = []; // {name, root}
const stmtProblems = [];
for (const [name, Cls] of Object.entries(statements)) {
  if (NON_GRAMMAR.has(name) || typeof Cls !== "function") continue;
  try {
    const inst = new Cls();
    const m = inst.getMatcher();
    stmtOut.push({ name, first: m.first(), root: walk(m) });
  } catch (e) {
    stmtProblems.push({ name, error: String(e.message ?? e) });
  }
}
const exprOut = [];
const exprProblems = [];
// exprQueue grows while we walk, so index loop
for (let i = 0; i < exprQueue.length; i++) {
  const [name, inst] = exprQueue[i];
  try {
    exprOut[exprIds.get(name)] = { name, root: walk(inst.getRunnable()) };
  } catch (e) {
    exprProblems.push({ name, error: String(e.message ?? e) });
    exprOut[exprIds.get(name)] = { name, root: mk({ k: "custom", cls: "ERROR" }) };
  }
}
// expressions never referenced by any statement are still part of the grammar (reported, not emitted)
const unreferenced = Object.keys(expressions).filter(
  (n) => typeof expressions[n] === "function" && !exprIds.has(n) && n !== "Expression",
);

// ---- coverage ----
function reachableCustom(rootId, seen = new Set(), exprSeen = new Set()) {
  const stack = [rootId];
  const out = new Set();
  while (stack.length) {
    const id = stack.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    const n = nodes[id];
    if (n.k === "custom") out.add(n.cls);
    else if (n.k === "expr") {
      if (!exprSeen.has(n.e)) {
        exprSeen.add(n.e);
        stack.push(exprOut[n.e].root);
      }
    } else if (Array.isArray(n.c)) stack.push(...n.c);
    else if (typeof n.c === "number") stack.push(n.c);
  }
  return out;
}
const exprDecl = exprOut.filter((e) => reachableCustom(e.root).size === 0).length;
const stmtDecl = stmtOut.filter((s) => reachableCustom(s.root).size === 0).length;

// Statement-level behaviour that lives outside the combinators (statement_parser.js & friends).
const parserDir = path.join(src, "abap/2_statements");
const procedural = ["statement_parser.js", "expand_macros.js", "_select_reclassify.js"].map((f) => ({
  file: f,
  lines: fs.readFileSync(path.join(parserDir, f), "utf8").split("\n").length,
}));

const kinds = {};
for (const n of nodes) kinds[n.k] = (kinds[n.k] ?? 0) + 1;

const report = {
  abaplint: pkgVersion,
  statements: { total: stmtOut.length, declarative: stmtDecl, failed: stmtProblems },
  expressions: {
    reachable: exprOut.length,
    declarative: exprDecl,
    unreferencedByStatements: unreferenced,
    failed: exprProblems,
  },
  customNodeKinds: Object.fromEntries(customKinds),
  nodeKinds: kinds,
  uniqueNodes: nodes.length,
  distinctRegexes: [...regexes].sort(),
  tokenKinds: [...tokenNames].sort(),
  proceduralFilesOutsideCombinators: procedural,
};

fs.mkdirSync(outDir, { recursive: true });
const ir = {
  abaplint: pkgVersion,
  releases: version.ReleaseList.map((r) => r.name),
  statements: stmtOut,
  expressions: exprOut,
  nodes,
};
fs.writeFileSync(path.join(outDir, "grammar.json"), JSON.stringify(ir));
fs.writeFileSync(path.join(outDir, "report.json"), JSON.stringify(report, null, 2) + "\n");
// ---- Rust table ----
const rs = [];
const q = (x) => JSON.stringify(x);
const ident = (n) => n;
rs.push('// @generated by tools/gen-grammar/gen.cjs from @abaplint/core ' + pkgVersion + ' (MIT). Do not edit.');
rs.push('use crate::{Node, Stmt};');
rs.push('#[derive(Clone, Copy, Debug, PartialEq, Eq)]');
rs.push('pub enum Tk { ' + [...tokenNames, 'Other'].sort().join(', ') + ' }');
rs.push('impl Tk {');
rs.push('    pub fn from_name(n: &str) -> Option<Tk> {');
rs.push('        Some(match n {');
for (const t of [...tokenNames].sort()) rs.push('            ' + q(t) + ' => Tk::' + t + ',');
rs.push('            _ => return None,');
rs.push('        })');
rs.push('    }');
rs.push('}');
rs.push('pub const ABAPLINT_VERSION: &str = ' + q(pkgVersion) + ';');
const nodeSrc = (n) => {
  const ids = (a) => '&[' + a.join(',') + ']';
  switch (n.k) {
    case 'word': return 'Node::Word(' + q(n.s) + ')';
    case 'regex': return 'Node::Regex(' + q(n.re) + ',' + n.flags.includes('i') + ')';
    case 'tok': return 'Node::Tok(Tk::' + ident(n.name) + ')';
    case 'expr': return 'Node::Expr(' + n.e + ')';
    case 'seq': case 'alt': case 'altPrio': case 'per':
      return 'Node::' + {seq:'Seq',alt:'Alt',altPrio:'AltPrio',per:'Per'}[n.k] + '(' + ids(n.c) + ')';
    case 'opt': case 'optPrio': case 'star': case 'starPrio': case 'plus': case 'plusPrio':
      return 'Node::' + {opt:'Opt',optPrio:'OptPrio',star:'Star',starPrio:'StarPrio',plus:'Plus',plusPrio:'PlusPrio'}[n.k] + '(' + n.c + ')';
    case 'ver': return 'Node::Ver(' + n.rel + ',' + n.openAbap + ',' + n.c + ')';
    case 'verLang': return 'Node::VerLang(' + ({KeyUser:0,Cloud:1,Normal:2}[n.lang]) + ',' + n.c + ')';
    case 'verNotLang': return 'Node::VerNotLang(' + ({KeyUser:0,Cloud:1,Normal:2}[n.lang]) + ',' + n.c + ')';
    case 'fail': return 'Node::Fail';
    case 'failStar': return 'Node::FailStar';
    case 'stopBefore1': return 'Node::StopBefore1(&[' + n.w.map(q).join(',') + '])';
    case 'stopBefore2': return 'Node::StopBefore2(' + q(n.w[0]) + ',' + q(n.w[1]) + ')';
    default: throw new Error('unhandled node ' + n.k);
  }
};
rs.push('pub static NODES: [Node; ' + nodes.length + '] = [');
for (const n of nodes) rs.push('    ' + nodeSrc(n) + ',');
rs.push('];');
rs.push('pub static EXPRESSIONS: [(&str, u32); ' + exprOut.length + '] = [');
for (const e of exprOut) rs.push('    (' + q(e.name) + ', ' + e.root + '),');
rs.push('];');
rs.push('pub static STATEMENTS: [Stmt; ' + stmtOut.length + '] = [');
for (const s of stmtOut) rs.push('    Stmt { name: ' + q(s.name) + ', first: &[' + s.first.map(q).join(',') + '], root: ' + s.root + ' },');
rs.push('];');
rs.push('pub static RELEASES: [&str; ' + ir.releases.length + '] = [' + ir.releases.map(q).join(',') + '];');
fs.writeFileSync(path.join(outDir, "grammar_tables.rs"), rs.join("\n") + "\n");
fs.writeFileSync(path.join(outDir, "token_kinds.txt"), [...tokenNames].sort().join("\n") + "\n");
console.log(JSON.stringify(report, null, 2));
