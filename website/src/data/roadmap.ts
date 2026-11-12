// Milestones, from docs/roadmap.md. Update both together.

export type Status = 'done' | 'active' | 'planned';

export interface Step {
  text: string;
  done: boolean;
}

export interface Milestone {
  id: string;
  title: string;
  status: Status;
  summary: string;
  exit: string;
  steps?: Step[];
}

export const milestones: Milestone[] = [
  {
    id: 'M0',
    title: 'Design',
    status: 'done',
    summary: 'The language reference, the grammar, and example programs.',
    exit: 'Documents and examples exist.',
  },
  {
    id: 'M1',
    title: 'Hello world',
    status: 'done',
    summary: 'Lexer and parser for the full grammar, diagnostics with codes and JSON, a Cranelift JIT, `lucid run` and `lucid check`.',
    exit: '`lucid run` prints `Hello, world!`, and every example parses.',
  },
  {
    id: 'M2',
    title: 'Core language',
    status: 'done',
    summary: 'Numbers with overflow traps, `let` and `var`, `if`, `while`, `for`, tasks and recursion, and type checking.',
    exit: 'FizzBuzz and recursive Fibonacci run.',
  },
  {
    id: 'M3',
    title: 'Data',
    status: 'done',
    summary: 'Text, lists, maps, sets, `type`, `choice`, `match`, unions and narrowing, generics, traits, `any`, lambdas, parameter modes, and reference counting.',
    exit: '`examples/hello.lcd` runs, and golden tests cover every feature with no leaks.',
  },
  {
    id: 'M4',
    title: 'Ownership performance',
    status: 'done',
    summary: 'Moves on last use, static text, fewer count operations, `lucid check --costs`, and a benchmark suite.',
    exit: 'The benchmark suite is measured against the speed and memory targets.',
  },
  {
    id: 'M5',
    title: 'A usable language',
    status: 'done',
    summary: '`try` and `else`, tests, `unsafe`, modules, packages, permissions, the first `std` modules, and the `build`, `test`, `explain`, `context`, `fix`, and `fmt` commands.',
    exit: '`examples/todo.lcd` builds to a standalone `.exe` and works.',
  },
  {
    id: 'M6',
    title: 'Browser',
    status: 'done',
    summary: 'A WebAssembly backend, `lucid build --target web`, and `std.dom` for interactive pages.',
    exit: 'An interactive page written only in Lucid runs in a browser.',
  },
  {
    id: 'M7',
    title: 'Concurrency and servers',
    status: 'done',
    summary: 'Bit operators, threads with `spawn`, channels, locks and a deadlock trap, `std.net`, an HTTP/1.1 server and client with TLS, and `std.html`.',
    exit: '`examples/website.lcd` serves pages under a load test.',
  },
  {
    id: 'M8',
    title: 'Ecosystem',
    status: 'done',
    summary: 'Linux, cryptography, SQLite, a package registry, a language server, and the 1.0 stability promise.',
    exit: 'A package that uses a registry package with `std.sql` and `std.crypto` builds and passes `lucid test` on Windows and Linux, and an editor shows its diagnostics through `lucid lsp`.',
    steps: [
      { text: 'Prelude text tasks: `starts_with`, `ends_with`, `contains`, `find`, `trim`', done: true },
      { text: 'Compile only reachable tasks', done: true },
      { text: 'Linux x86-64: `run`, `test`, and `build`', done: true },
      { text: '`std.crypto`', done: true },
      { text: '`std.sql` and the `sql` type', done: true },
      { text: 'Orphan rule for tasks with `self`', done: true },
      { text: 'Registry, `lucid package`, version selection, and `lucid.lock`', done: true },
      { text: '`lucid lsp`', done: true },
      { text: 'Spec review, then the version 1.0 promise', done: true },
    ],
  },
  {
    id: 'M9',
    title: 'Self-hosted front end',
    status: 'done',
    summary: 'The lexer, parser, checker, and ownership pass ported to Lucid, one stage at a time, each checked against the Rust stage. Code generation stays in Rust.',
    exit: 'The compiler written in Lucid builds itself, and a second build is byte-identical.',
    steps: [
      { text: '`LUCID_DUMP` stage dumps and a test that compares both compilers', done: true },
      { text: 'Lexer and parser in Lucid', done: true },
      { text: 'Text append in place through a `change` parameter or a field', done: true },
      { text: 'Resolver in Lucid', done: true },
      { text: 'Type checker in Lucid', done: true },
      { text: '`derive` and the monomorphizer in Lucid', done: true },
      { text: 'Ownership pass, `--costs`, and `--unsafe` in Lucid', done: true },
      { text: 'Rust backend behind a small C ABI: the Lucid compiler runs `check`, `run`, and `build`', done: true },
      { text: 'Fixed point of three builds, and the bootstrap path', done: true },
    ],
  },
  {
    id: 'M10',
    title: 'Release readiness',
    status: 'active',
    summary: 'Version 1.0.0: a deep review and its fixes, a version flag, smaller Linux programs, a Firefox check of the web loader, and written release steps.',
    exit: '`lucid --version` prints `lucid 1.0.0` on Windows and Linux, and every test passes on both.',
    steps: [
      { text: 'Deep review of the M9 work, and its fixes', done: true },
      { text: 'Version 1.0.0 and `lucid --version`', done: true },
      { text: 'Linux programs without debug sections', done: true },
      { text: 'Web loader checked in Firefox', done: false },
      { text: 'Release steps, with a hash of each binary', done: false },
      { text: 'License review', done: false },
      { text: 'First public release', done: false },
    ],
  },
];

export const later =
  'Not planned yet: macOS, the Lucid compiler for every command, `shared` and `weak`, an LLVM release backend, plain types stored inline, and compile-time evaluation.';
