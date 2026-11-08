// Runs `lucid check` on every lucid code block in the docs and on every
// file in src/examples. Needs a built compiler: LUCID, or ../target/release/lucid[.exe].
import { readFileSync, readdirSync, writeFileSync, mkdtempSync, existsSync, rmSync } from 'node:fs';
import { join, relative } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';

const root = new URL('..', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
const lucid = process.env.LUCID ?? ['lucid.exe', 'lucid'].map((n) => join(root, '..', 'target', 'release', n)).find(existsSync);
if (!lucid) {
  console.error('No compiler found. Build it with `cargo build --release`, or set LUCID.');
  process.exit(2);
}

const walk = (dir) => readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)]));
const samples = [];
for (const file of walk(join(root, 'src', 'content', 'docs'))) {
  if (!/\.mdx?$/.test(file) || file.endsWith('diagnostics.md')) continue; // diagnostics shows errors on purpose
  const text = readFileSync(file, 'utf-8').replace(/\r\n/g, '\n');
  for (const m of text.matchAll(/^```lucid[^\n]*\n([\s\S]*?)^```$/gm)) {
    const line = text.slice(0, m.index).split('\n').length;
    samples.push({ where: `${relative(root, file)}:${line}`, code: m[1] });
  }
}
// Showcase files must also be in the one layout, since the site says there is one.
for (const file of walk(join(root, 'src', 'examples'))) {
  samples.push({ where: relative(root, file), code: readFileSync(file, 'utf-8') });
  const r = spawnSync(lucid, ['fmt', '--check', file], { encoding: 'utf-8' });
  if (r.status !== 0) {
    console.error(`NOT FORMATTED ${relative(root, file)}: run lucid fmt on it`);
    process.exitCode = 1;
  }
}

const dir = mkdtempSync(join(tmpdir(), 'lucid-site-'));
let failed = 0;
samples.forEach((s, i) => {
  const path = join(dir, `sample${i}.lcd`);
  writeFileSync(path, s.code);
  const r = spawnSync(lucid, ['check', path], { encoding: 'utf-8' });
  if (r.status !== 0) {
    failed++;
    console.error(`FAIL ${s.where}\n${(r.stdout + r.stderr).replaceAll(path, s.where)}`);
  }
});

// The home page shows one diagnostic as the compiler prints it. Check that it still does.
const d = JSON.parse(readFileSync(join(root, 'src', 'data', 'diagnostic.json'), 'utf-8'));
const fixed = readFileSync(join(root, 'src', 'examples', d.file), 'utf-8').replace(/\r\n/g, '\n');
const broken = fixed.replace(`${d.removed}\n`, '');
const shown = [
  ['run', fixed, d.run],
  ['check', broken, d.check],
  ['check --json', broken, d.json],
];
for (const [args, code, want] of shown) {
  writeFileSync(join(dir, d.file), code);
  const r = spawnSync(lucid, [...args.split(' '), d.file], { cwd: dir, encoding: 'utf-8' });
  const got = (r.stdout + r.stderr).replace(/\r\n/g, '\n').trimEnd();
  if (got !== want) {
    failed++;
    console.error(`FAIL home page diagnostic, lucid ${args}:\nwant ${want}\ngot  ${got}`);
  }
}
rmSync(dir, { recursive: true, force: true });
console.log(`${samples.length} samples and 3 home page outputs, ${failed} failed`);
if (failed) process.exitCode = 1;
