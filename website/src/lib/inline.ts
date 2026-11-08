import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const escape = (s: string) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

/** Escape text, and turn `code` spans into <code> elements. */
export const inline = (s: string) => escape(s).replace(/`([^`]+)`/g, '<code>$1</code>');

/** Read a file of the repository. The build runs in website/, locally and on Cloudflare. */
export const repoFile = (rel: string) =>
  readFileSync(resolve(process.cwd(), '..', rel), 'utf-8').replace(/\r\n/g, '\n');
