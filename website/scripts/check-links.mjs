// Checks that every internal href and src in dist/ points at a built file.
import { readFileSync, readdirSync, existsSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const dist = new URL('../dist', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
const walk = (dir) => readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)]));
const resolves = (url) => {
  const p = join(dist, decodeURIComponent(url));
  return (existsSync(p) && statSync(p).isFile()) || existsSync(join(p, 'index.html')) || existsSync(`${p}.html`);
};

const pages = walk(dist).filter((f) => f.endsWith('.html'));
let checked = 0;
const broken = [];
for (const page of pages) {
  const html = readFileSync(page, 'utf-8');
  for (const [, url] of html.matchAll(/\s(?:href|src)="(\/(?!\/)[^"#?]*)/g)) {
    checked++;
    if (!resolves(url)) broken.push(`${relative(dist, page)}: ${url}`);
  }
}
console.log(`${pages.length} pages, ${checked} internal links, ${broken.length} broken`);
for (const b of broken) console.error(`BROKEN ${b}`);
process.exit(broken.length ? 1 : 0);
