# Site 1 — Astro marketing site with AI-agent-friendly markdown

## IMPORTANT — process note

**Do not run `git commit` or `git push` under any circumstances.** Leave all changes in the
working tree. Do not run `vercel deploy`/`vercel link` — deployment is handled separately, by the
orchestrator, against a real live account. Print the required `DONE_MARKER` string when finished.

## Scope

Build a new Astro marketing/landing site for **Djangors**, a Django-inspired Rust web framework
(the parent project, at the repo root — read `README.md` in full first, it's the primary content
source). This is a **new site alongside the existing mdBook docs** (`docs/src/*.md` → `docs/book/`
via `mdbook build docs`) — do not touch the mdBook source or its build config, and do not attempt
to migrate the tutorial/topic-guides content into this site.

**Astro version**: use current Astro (`npm create astro@latest`, latest stable — confirmed
`7.1.4` as of this doc, but verify whatever `npm create astro@latest` actually installs and use
its real, current API rather than an older/remembered content-collections shape — Astro's content
collections API has changed across major versions; check `node_modules/astro`'s own
documentation/types or astro.build's current docs for the real current API before writing content
collection code, don't guess from general familiarity).

## Real assets to use (already in the repo — do not regenerate or fabricate replacements)

- `assets/logo.svg` — the real Djangors logo. Use it directly for the site header/hero.
- `assets/favicons/` — a complete, already-generated favicon set: `favicon.ico`,
  `favicon-16x16.png`, `favicon-32x32.png`, `apple-touch-icon.png`,
  `android-chrome-192x192.png`, `android-chrome-512x512.png`. Copy these into the Astro project's
  `public/` directory and wire up the real `<link rel="icon">`/`<link rel="apple-touch-icon">`
  tags and a real `site.webmanifest` (write fresh JSON with real Djangors values — `name:
  "Djangors"`, `short_name: "Djangors"`, `theme_color: "#f27822"`, `background_color: "#f5f1e7"`,
  `display: "standalone"` — the icons array pointing at the two android-chrome PNGs above; do not
  copy any placeholder "Your App Name" content).

## Theme — real colors extracted directly from `assets/logo.svg`, not guessed

`#201e1e` (near-black charcoal), `#f27822` (bright orange, primary accent — thematically apt for a
Rust framework), `#d56020` (deeper burnt-orange, secondary accent), `#f5f1e7` (warm cream,
background). Build the site's CSS custom properties from these four colors, light/dark-mode aware
(`prefers-color-scheme`), matching how the rest of this project treats dark mode as a first-class
concern (the mdBook theme itself is already light/dark-aware).

## Content sourcing — copy real files at build time, never hand-retype

Write a small `scripts/sync-content.mjs` that runs before `astro build` (wire it as a `prebuild`
npm script) and copies, verbatim, from the parent repo into the Astro project's content directory:
- `../README.md`
- `../docs/src/django-comparison.md`
- `../docs/src/benchmarks.md`

This is the single source of truth — page copy must never be hand-retyped into `.astro` files,
since that would silently drift from the real docs. The same script also generates:
- `/llms.txt` at the Astro project's public root: a short index per the emerging llms.txt
  convention (project name/one-line summary/links to the key docs, not the full content).
- `/llms-full.txt`: the full real concatenation of `README.md` + every file under
  `../docs/src/**/*.md` (recursively) + `../PLAN.md`. Generate this from the actual current files
  every build — it must never be hand-maintained separately from the docs.

## Pages

- `/` — hero (logo, tagline/pitch pulled from the real README content, not rewritten), the real
  crate-by-crate feature table (from the README), the real benchmark numbers table (from
  `benchmarks.md`), CTA links to GitHub (`https://github.com/QUANTORA-ltd/djangors`) and to the docs
  (`/docs/` — see the mdBook-copy note below for why that path will work once deployed).
- `/compare` — renders the synced `django-comparison.md` content.
- `/benchmarks` — renders the synced `benchmarks.md` content.

## AI-markdown features (the actual point of this site)

- Every content page (`/`, `/compare`, `/benchmarks`) has a matching raw-markdown sibling route
  (e.g. `/compare` ⇄ `/compare.md`) serving the real synced markdown source verbatim — not a
  re-rendered/re-serialized copy. Implement as a plain Astro endpoint per page
  (`src/pages/compare.md.ts` returning `text/markdown`, reading the same synced content file the
  `.astro` page renders) — verify the real, current Astro endpoint API
  (`APIRoute`/`GetStaticPaths` or whatever the installed version actually uses) rather than
  assuming.
- A "Copy as Markdown" button component on every content page (client-side vanilla JS is fine, no
  framework needed — copies the same raw source the `.md` route serves, via
  `navigator.clipboard.writeText`).
- `/llms.txt` and `/llms-full.txt` per the content-sourcing section above.

## mdBook copy step (wire this into the Astro build, but do NOT write the mdBook-side script itself — that's handled separately)

Add a `prebuild` (or equivalent) step to the Astro project's own build process that runs `mdbook
build ../docs` and then copies the resulting `../docs/book/` directory into the Astro project's
`public/docs/` directory, so the built mdBook site ends up served at `/docs/*` in the same Vercel
deployment. (A separate, already-planned change will make `docs/book/` itself contain matching
`.md` files alongside the `.html` ones — you don't need to build that part, just make sure the
whole `docs/book/` directory, whatever it contains, gets copied into `public/docs/` intact,
including any `.md` files that already exist there by the time you run this.)

## Deliberately excluded

Migrating tutorial/topic-guides content into Astro/Starlight; a blog/CMS; server-rendered
personalization or any backend (this is a fully static site — set `output: "static"` or whatever
the current Astro config calls the static-only mode); actually running `vercel deploy` (handled
separately).

## Required verification

- `npm run build` (or `astro build`) succeeds in the new site directory.
- Confirm `/llms-full.txt`'s built output genuinely contains the real, current content of
  `README.md` and at least one real `docs/src/guides/*.md` file (grep for a distinctive real
  sentence from each source file inside the built output — don't just confirm the file exists).
- Confirm `/compare.md` (or wherever you place it) serves real markdown text (not HTML, not a 404)
  when the built site is served locally (`astro preview` or equivalent).
- `cargo fmt`/`clippy`/etc. are irrelevant here (this is a JS/TS project) — instead run whatever
  linting the Astro scaffold includes by default, and report the real output.
- Confirm the favicon/manifest files from `assets/favicons/` actually appear in the built output
  referenced correctly (check the built HTML's `<head>` for real `<link>` tags pointing at files
  that actually exist in the build output, not 404s).

Print `DONE_MARKER_SITE_1_ASTRO` when finished (or when stopping early — say exactly what works
and what doesn't, do not fabricate a finished result). Leave all changes in the working tree — do
not commit, push, or deploy.
