# lucidlang.dev

The public website for Lucid: a static Astro site, with Starlight for the docs. It builds alone, with no Rust toolchain, and deploys to Cloudflare Pages.

## Specification

**Audience.** Developers who meet Lucid for the first time, and agents that need to write it.

**Rules.**

- Every claim comes from `docs/` or the compiler, and says what is true now. Planned work is marked planned.
- No public release exists yet, and the source repository is private. The site says so, and offers no download or install command that cannot work.
- Content is curated. The site never publishes `docs/research.md`, `docs/decisions.md`, `docs/claude-code.md`, or internal compiler detail. Two files are read straight from `docs/`, since they are written for users: `errors.md`, for the diagnostics page, and `agent-guide.md`, served as `/agent-guide.md`. The Learn page shows the programs in `examples/` as they are.
- Every `lucid` code block in the docs is a whole program that passes `lucid check`. `npm run check:examples` proves it.
- No analytics, no tracking, no external fonts or scripts. JavaScript only for search, the theme toggle, the menu, and copy buttons.

**Routes.**

| Route | Content |
| --- | --- |
| `/` | Hero, code sample, features, status, getting started |
| `/features` | Each capability, with an example and its limits |
| `/learn` | A guided path through the docs, and the example programs |
| `/roadmap` | Milestones M0 to M9, from `docs/roadmap.md` |
| `/downloads` | Release status, supported platforms, the planned install design |
| `/releases` | Redirects to `/downloads` until a release exists |
| `/docs/...` | Starlight docs, with search from Pagefind |
| `/agent-guide.md` | `docs/agent-guide.md`, as plain text |
| `/install.sh`, `/install.ps1` | Reserved. Each prints that no release exists, and fails |
| `/404` | Not found |

**Design.** Dark first, near-black, one accent (ice cyan), IBM Plex Sans and Mono, self-hosted. A fine grid on the hero only. A Lucid TextMate grammar in `src/lucid.tmLanguage.json` colors code on every page.

## Commands

Run from `website/`, with Node 22.23 or later:

```sh
npm ci                    # install
npm run dev               # local server at http://localhost:4321
npm run build             # sync diagnostics, type check, build to dist/
npm run preview           # serve dist/
npm run check:examples    # lucid check on every lucid block in the docs
npm run check:links       # every internal link in dist/ resolves
```

`check:examples` needs a built compiler. It uses `../target/release/lucid.exe` or `../target/release/lucid`, or the path in `LUCID`.

## Cloudflare Pages

Checked against the Cloudflare Pages docs on 2026-11-08. In the dashboard: Workers & Pages, Create application, Pages, Import an existing Git repository. Pick this repository, then Begin setup:

| Setting | Value |
| --- | --- |
| Production branch | `main` |
| Framework preset | Astro, or None: the fields below are what matter |
| Root directory | `website` |
| Build command | `npm run build` |
| Build output directory | `dist` |
| Environment variables | none. `.node-version` pins Node 22.23.4, the v3 build image's default |

Then Save and Deploy. The site is fully static, so the free plan covers it, and the build needs no token or secret.

The build reads `../docs/errors.md` and `../docs/agent-guide.md`, and `../examples/`. The root directory only sets where the build runs, and the repository is cloned whole, but the Cloudflare docs do not say so outright: check the first deploy's log.

`public/_headers` sets content types for the install scripts and the agent guide. `dist/404.html` is served for unknown paths.

**Custom domain.** Cloudflare requires the apex domain `lucidlang.dev` to be a zone on the same Cloudflare account, so first add the domain to Cloudflare and point its nameservers there. Then open the Pages project, Custom domains, Set up a domain, enter `lucidlang.dev`, and continue: Cloudflare adds the DNS record. `www.lucidlang.dev` can be added the same way, as a `CNAME` to the project's `pages.dev` address.

## Planned install design

Not built. Installers go live only with a real release.

- A release publishes `https://lucidlang.dev/releases.json`: for each version, each supported target (`x86_64-windows`, later `x86_64-linux`), the HTTPS URL of its archive, and its SHA-256.
- `install.sh` and `install.ps1` read that file, pick the target, download over HTTPS, and stop with a clear message if the platform is unsupported, the download fails, or the hash differs. They run nothing they download.
- They install to a user directory with no administrator rights: `~/.lucid/bin`, or `%LOCALAPPDATA%\Lucid\bin`. They print the line to add it to `PATH`, and change no shell profile by themselves.
- Running one again installs the latest version over the old one, and does nothing if it is current.
