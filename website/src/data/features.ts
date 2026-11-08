// Each claim here is checked against docs/ and the compiler. Code lives in src/examples/.
import readable from '../examples/readable.lcd?raw';
import values from '../examples/values.lcd?raw';
import safety from '../examples/safety.lcd?raw';
import explicit from '../examples/explicit.lcd?raw';
import counter from '../examples/counter.lcd?raw';

export interface Feature {
  id: string;
  title: string;
  short: string;
  body: string[];
  limits?: string;
  code?: { file: string; source: string };
  terminal?: { cmd?: string; out?: string }[];
  docs: { href: string; label: string };
}

export const features: Feature[] = [
  {
    id: 'readable',
    title: 'Readable by default',
    short: 'Blocks are four-space indentation. Functions are `task`, bindings are `let` and `var`, and one formatter has no options.',
    body: [
      'A statement ends at the end of its line, and a block is indented by exactly four spaces. There are no semicolons or braces around blocks, and no truthiness: a condition must be a `bool`.',
      'Each common job has one form. Methods are tasks whose first parameter is `self`, with no classes or `impl` blocks. `lucid fmt` puts every file in one layout, with no settings, so a diff shows only meaning.',
    ],
    code: { file: 'readable.lcd', source: readable },
    docs: { href: '/docs/syntax/', label: 'Syntax' },
  },
  {
    id: 'values',
    title: 'Inferred ownership, value semantics',
    short: 'Every value is independent. Changing a copy never changes the original, and you write no lifetimes.',
    body: [
      '`var b = a` gives `b` its own logical value. There are no stored references, so nothing can dangle, and no lifetime annotations exist.',
      'The compiler keeps logical copies cheap: a value used for the last time is moved, a shared buffer is counted, and a write copies once only if the buffer is shared. Parameters are read-only views, so passing a value never copies it.',
    ],
    limits: 'Heavily shared data pays for reference counting. `lucid check --costs` lists every copy and count site, so you can see the cost.',
    code: { file: 'values.lcd', source: values },
    docs: { href: '/docs/ownership/', label: 'Ownership and memory' },
  },
  {
    id: 'safety',
    title: 'Memory safety without a garbage collector',
    short: 'No null, bounds checks, and integer overflow that traps in every build. Safe code has no undefined behavior.',
    body: [
      'Memory is freed by reference counts that the compiler places, so there is no collector and no pause. Absence is `none`, and a `T?` must be narrowed before use.',
      'An out-of-bounds index, a missing map key, overflow, or a zero divisor is a trap: a controlled stop with the source position and a stack trace, never silent corruption.',
    ],
    limits: 'A reference cycle leaks unless it uses `weak`, which is not built yet. Inside an `unsafe` block, the guarantees are the author’s job.',
    code: { file: 'safety.lcd', source: safety },
    docs: { href: '/docs/safety/', label: 'Safety' },
  },
  {
    id: 'native',
    title: 'Native code through Cranelift',
    short: '`lucid run` compiles with a JIT and runs at once. `lucid build` writes a standalone executable.',
    body: [
      'There is no interpreter. `lucid run` compiles the program to machine code in memory with Cranelift and runs it. `lucid build` writes the same code to a standalone program, with the runtime linked in.',
      'Today `lucid build` makes a Windows x86-64 `.exe`. Linux x86-64 is the next step of M8.',
    ],
    limits: 'Cranelift compiles fast but optimizes less than LLVM, so some tight numeric loops run slower than in Rust or C++.',
    terminal: [
      { cmd: 'lucid run hello.lcd' },
      { out: 'Hello, world' },
      { cmd: 'lucid build hello.lcd' },
      { cmd: './hello.exe' },
      { out: 'Hello, world' },
    ],
    docs: { href: '/docs/cli/', label: 'CLI reference' },
  },
  {
    id: 'web',
    title: 'WebAssembly for the browser',
    short: '`lucid build --target web` writes one `.js` file that runs the program in a browser or under Node.',
    body: [
      'The same program compiles to WebAssembly, with its runtime, in a single `.js` file that loads both modules. A page includes it with a `<script>` tag.',
      '`std.dom` builds interactive pages in the Elm architecture: a model, a `view` task, and an `update` task that turns each event into a new model.',
    ],
    limits: 'On the web, `spawn`, `std.net`, `std.http`, and foreign calls are compile errors, since a page has no threads, sockets, or C library.',
    code: { file: 'counter.lcd', source: counter },
    docs: { href: '/docs/webassembly/', label: 'WebAssembly' },
  },
  {
    id: 'explicit',
    title: 'Explicit errors, mutation, and concurrency',
    short: '`try` marks an early exit, `change` marks mutation, and `spawn` marks a new thread, at the call site.',
    body: [
      'There are no exceptions. A task that can fail returns a union with an error type, and an error that nothing uses does not compile. `try` passes it up, `else` gives a fallback, and `match` handles each case.',
      'A value sent to another thread is a copy, so safe code has no data races. Threads share state only through a `channel` or a `lock`, and when every thread waits, the program traps with a deadlock message.',
    ],
    code: { file: 'explicit.lcd', source: explicit },
    docs: { href: '/docs/errors/', label: 'Error handling' },
  },
  {
    id: 'agents',
    title: 'Diagnostics and tools for agents',
    short: 'Every command has `--json` output. Every error has a stable code, an explanation, and often a fix.',
    body: [
      'Errors come with stable codes that never change meaning. `lucid explain` prints the cause, an example, and a fix for each one, and `lucid fix` applies the fixes the compiler is sure of.',
      '`lucid context` prints the public API of a module with its doc comments, and a single guide holds everything that runs today, sized to fit in an agent’s context.',
    ],
    limits: 'Agent-friendliness is not proven. Models have no Lucid training data, so the guide and the diagnostics carry the whole load.',
    terminal: [
      { cmd: 'lucid check typo.lcd' },
      { out: 'typo.lcd:2:7: error E0301: name `totl` is not defined' },
      { out: '  fix: rename to `total`' },
      { cmd: 'lucid fix typo.lcd' },
      { out: 'typo.lcd:2:7: fixed E0301: rename to `total`' },
    ],
    docs: { href: '/docs/ai-agents/', label: 'Writing Lucid with agents' },
  },
];
