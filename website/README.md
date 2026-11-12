# lucidlang.dev

The public website for Lucid: a static Astro site, with Starlight for the docs. It builds alone, with no Rust toolchain. It is live at https://lucidlang.dev, served by Cloudflare Pages.

## Specification

**Audience.** Developers who meet Lucid for the first time, and agents that need to write it.

**Rules.**

- Every claim comes from `docs/` or the compiler, and says what is true now. Planned work is marked planned.
- Version 1.0.0 is released, with binaries for Windows and Linux on x86-64 as assets of the GitHub release. The site offers no download or install command that cannot work.
- Content is curated. The site never publishes `docs/research.md`, `docs/decisions.md`, `docs/claude-code.md`, or internal compiler detail. Two files are read straight from `docs/`, since they are written for users: `errors.md`, for the diagnostics page, and `agent-guide.md`, served as `/agent-guide.md`. The Learn page shows the programs in `examples/` as they are.
- Every `lucid` code block in the docs is a whole program that passes `lucid check`. `npm run check:examples` proves it.
- No analytics, no tracking, no external fonts or scripts. JavaScript only for search, the theme toggle, the menu, and copy buttons.

**Routes.**

| Route | Content |
| --- | --- |
| `/` | Hero, code sample, features, status, getting started |
| `/features` | Each capability, with an example and its limits |
| `/learn` | A guided path through the docs, and the example programs |
| `/roadmap` | Milestones M0 to M10, from `docs/roadmap.md` |
| `/downloads` | The installers, the archives with their hashes, and the platforms |
| `/releases` | Redirects to `/downloads` |
| `/releases.txt` | The release list the installers read: `VERSION PLATFORM URL SHA256` per line, newest first. Update it with each release |
| `/docs/...` | Starlight docs, with search from Pagefind |
| `/agent-guide.md` | `docs/agent-guide.md`, as plain text |
| `/install.sh`, `/install.ps1` | The installers. Each reads `/releases.txt`, checks the archive's SHA-256, and installs into the user's directory |
| `/404` | Not found |

**Design.** Dark first: a graphite ground in three steps, Host Grotesk for reading and Atkinson Hyperlegible Mono for code and compiler output, both self-hosted. Amber marks what a person does (the primary action, the current milestone), and cool blue what the compiler wrote. Tokens are in `src/styles/tokens.css`, and `docs.css` restates them for Starlight. The home page leads with one real diagnostic: `src/examples/shapes.lcd` with one arm removed, and the outputs in `src/data/diagnostic.json`, which `check:examples` compares with the compiler. A Lucid TextMate grammar in `src/lucid.tmLanguage.json` colors code on every page.

## Commands

Run from `website/`, with Node 22.23 or later:

```sh
npm ci                    # install
npm run dev               # local server at http://localhost:4321
npm run build             # sync diagnostics, type check, build to dist/
npm run preview           # serve dist/
npm run check:examples    # lucid check on every lucid block, and the home page diagnostic
npm run check:links       # every internal link in dist/ resolves
```

`check:examples` needs a built compiler. It uses `../target/release/lucid.exe` or `../target/release/lucid`, or the path in `LUCID`.

## Deployment

Cloudflare Pages builds and publishes the site from `main` of this
repository, with `website/` as the root directory, `npm run build` as the
command, and `dist` as the output. It needs no token or secret. Every
push to `main` is live within minutes, so run the checks under
"Commands" first.

`site` in `astro.config.mjs` is `https://lucidlang.dev`. Canonical links,
`og:url`, `og:image`, the sitemap, and `robots.txt` all build from it.
Never write any other address of the site in the site or its docs.

The build reads `../docs/errors.md`, `../docs/agent-guide.md`, and
`../examples/`, since Pages clones the whole repository. `public/_headers`
sets content types for the install scripts, the release list, and the
agent guide.

## Installers

`public/install.sh` and `public/install.ps1` read `public/releases.txt`,
and follow decision 233 in `docs/decisions.md`. A release adds its lines
to `releases.txt`, as the release steps in `docs/compiler.md` say.
