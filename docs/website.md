# Public project website

The English and Italian project website lives in `website/`, alongside the
application. It is a static site: publishing it does not run the Lekton server or
expose its database, authentication or ingestion endpoints.

## Build and preview

Use Node.js 22 or later. The builder and preview server use only Node's standard
library; building the website needs neither npm dependencies nor Rust services.

```bash
npm run build:website
npm run dev:website
```

Open <http://localhost:4173/lekton/> for English or
<http://localhost:4173/lekton/it/> for Italian. The preview uses the project prefix
to exercise the same relative URLs as GitHub Pages. It binds to localhost and
rebuilds on startup; restart it after editing source files. To choose another port,
use `npm run dev:website -- --port 4174`.

The generated `website/dist/` directory is ignored by Git. It contains static
HTML for both languages, shared assets and `.nojekyll`. Language links and native
expandable content work without JavaScript. JavaScript adds theme selection and
command copying; the system theme still applies when JavaScript is unavailable.

## Content and visual identity

- `website/content.mjs` contains both translations. The build rejects mismatched
  translation keys and missing template values.
- `website/index.html` defines the shared semantic structure.
- `website/site.css` owns the website layout and responsive rules.
- `website/theme.mjs` extracts the default light/dark colors, font families,
  radii and font import directly from `style/tailwind.css`. Change the application
  tokens to update both surfaces; do not add a second website palette.
- `website/theme-init.js` applies the theme before paint. `website/site.js` uses
  the application's system → light → dark cycle and `lekton-theme` storage key.

The portal illustration is HTML rather than an application screenshot. Its
caption labels it as illustrative; its navigation and search field are static.
Keep examples and claims aligned with the implementation and configuration.
In particular, search and RAG require their feature flags and configured services.
The local setup command selects Lekton and the demo loader plus their dependencies,
avoiding the optional Infinity model download. This is an evaluation environment;
the operations guide covers production configuration.

See [DESIGN.md](../DESIGN.md) for the shared visual rules. Approved compositions
and their generation provenance live in `.impeccable/mocks/`; local review captures
and build measurements are ignored by Git and are not website assets.

## Verification

```bash
npm run test:website
npm ci
npx playwright install chromium
npm run test:website:e2e
```

Unit tests verify token extraction, build failures and generated locale output.
Browser tests start the static preview themselves and check both languages and
themes, WCAG AA with axe, mobile overflow, links under `/lekton/`, native content
without JavaScript, theme persistence and clipboard/storage failures. They also
write desktop and mobile review captures to `.impeccable/review/`.

## Publishing with GitHub Pages

The [Project Website workflow](../.github/workflows/website.yml) runs the static
build tests and browser checks for website-related pull requests. On `main`, it
also uploads `website/dist/` and deploys that checked artifact through the
`github-pages` environment. Manual runs on feature branches validate without
publishing. Changes to the default application stylesheet rebuild the website
as well. All Actions are pinned to commit hashes.

For the initial publication, enable GitHub Pages for `dghilardi/lekton` and choose
**GitHub Actions** as its publishing source under **Settings → Pages**. Protect the
`github-pages` environment so only `main` can deploy. Then merge and push the
website branch to `main`, or run **Project Website** manually on `main` after the
source is configured. See GitHub's [custom workflow guide](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages)
for the publishing-source and deployment settings.

With the default project URL, English is published at
`https://dghilardi.github.io/lekton/` and Italian at
`https://dghilardi.github.io/lekton/it/`. These are deployment destinations, not a
claim that Pages has already been enabled. No `gh-pages` branch, Rust build or
application service credentials are needed. The workflow receives deployment
permissions only in its deployment job; pull request checks have read access.
