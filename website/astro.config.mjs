// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import lucid from './src/lucid.tmLanguage.json' with { type: 'json' };
import lucidDark from './src/lucid-dark.theme.json' with { type: 'json' };

const lucidLang = /** @type {any} */ ({ ...lucid, aliases: ['lcd'] });

export default defineConfig({
  site: 'https://lucidlang.dev',
  trailingSlash: 'always',
  redirects: { '/releases': '/downloads/' },
  markdown: {
    shikiConfig: { theme: /** @type {any} */ (lucidDark), langs: [lucidLang] },
  },
  integrations: [
    starlight({
      title: 'Lucid',
      description: 'Lucid is a readable, memory-safe, compiled programming language designed to be written reliably by people and AI agents.',
      logo: { src: './src/assets/mark.svg', alt: '' },
      favicon: '/favicon.svg',
      disable404Route: true,
      head: [
        { tag: 'meta', attrs: { property: 'og:image', content: 'https://lucidlang.dev/og.png' } },
        { tag: 'meta', attrs: { name: 'twitter:card', content: 'summary_large_image' } },
        { tag: 'meta', attrs: { name: 'theme-color', content: '#07080a' } },
      ],
      customCss: ['./src/styles/fonts.css', './src/styles/docs.css'],
      components: { SocialIcons: './src/components/DocsNav.astro' },
      expressiveCode: {
        themes: [/** @type {any} */ (lucidDark), 'github-light'],
        shiki: { langs: [lucidLang] },
        styleOverrides: { borderRadius: '6px', codeFontFamily: "'Atkinson Hyperlegible Mono', ui-monospace, monospace" },
      },
      sidebar: [
        { label: 'Start', items: ['docs', 'docs/installation', 'docs/cli'] },
        {
          label: 'Language',
          items: [
            'docs/syntax',
            'docs/variables-and-types',
            'docs/control-flow',
            'docs/tasks',
            'docs/collections',
            'docs/types-and-choices',
            'docs/generics-and-traits',
            'docs/errors',
            'docs/ownership',
            'docs/concurrency',
            'docs/modules-and-packages',
          ],
        },
        { label: 'Platform', items: ['docs/standard-library', 'docs/webassembly', 'docs/safety'] },
        { label: 'Tooling', items: ['docs/diagnostics', 'docs/ai-agents'] },
      ],
    }),
  ],
});
