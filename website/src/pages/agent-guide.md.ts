import type { APIRoute } from 'astro';
import { repoFile } from '../lib/inline';

// The agent guide, as plain text, straight from docs/agent-guide.md.
export const GET: APIRoute = () =>
  new Response(repoFile('docs/agent-guide.md'), { headers: { 'Content-Type': 'text/markdown; charset=utf-8' } });
