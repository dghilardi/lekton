# Reader delivery and schema viewer follow-up — 7 November 2026

Branch: `feat/portal-viewers-ssr`. Baseline for this follow-up: `20c0f9a`, containing the earlier portal accessibility work. The PR targets main and includes that prerequisite history.

## Observed problems and fixes

- Registry labels such as `authentication-property-integration` extended beyond the sidebar. Shared navigation styles now wrap names inside the column.
- OpenAPI at 390×844 produced a 471px page. Document/version badges now wrap, schema headings can break, and code remains in its scroll container.
- The OpenAPI download button used rgb(27,27,27) on the dark surface. Scalar's link/search/button tokens now follow the existing portal palette. Theme changes update the existing viewer instance.
- Scalar operation anchors had click handlers but no href or keyboard activation. The adapter exposes equivalent button semantics and Enter/Space activation without replacing the library's navigation handlers.
- The actual copied AsyncAPI document `mqtt/directory` is rejected for missing channel parameter declarations. The integration preserves the validator diagnostics, adds a recovery explanation and opens the original source. It does not silently rewrite or waive an invalid specification. A separate browser fixture checks valid channel rendering in both themes without altering stored data.
- Tiptap's heavy bundle (384787 bytes on the observed installation) was eagerly included on reader pages. Only its small WASM bridge remains statically imported; the heavy editor bundle loads when entering the editor.
- Registry index and sidebar independently requested the same 12666-byte schema list. They now share a layout-scoped resource. Initial hydration makes no list request.
- Documentation HTML originally contained no rendered article. Authorized documents and schema data now resolve through request-scoped serializable resources, using async SSR for complete reader HTML. Hydration reuses the results rather than fetching them again.
- Schema version selection is derived from the resolved metadata and the per-schema user selection, not mutated during render. This prevents a redundant content read during hydration.

## Authentication and delivery boundaries

Existing server functions still enforce access levels. No process-global reader-data cache was introduced. Reader HTML is `private, no-store`. Client navigations use the shared auth retry orchestrator. A failed SSR session is recovered through auth bootstrap, then only failed unauthorized reads are retried. A deterministic browser test simulates successful refresh; it does not claim to test an external OAuth provider or actual refresh-token rotation.

The initial document is readable with JavaScript disabled. Interactive viewers progressively enhance an SSR-readable source. Successful initial reads do not need WASM to expose the content. The raw HTML can be larger than a shell-only response; fewer API round trips do not establish a bandwidth or latency percentage improvement. Production compression, cold-network timing, heap profiling and physical-device/screen-reader checks remain outside the measured coverage.

## Verification

- Both cargo-leptos frontend and server builds passed with one Cargo job, dev debug information disabled and incremental compilation disabled.
- 19 browser checks passed: preceding portal keyboard/theme regressions plus SSR HTML, no-JS reading, anonymous denial of a private document, shared schema reads, lazy editor loading, OpenAPI geometry/theme/keyboard navigation/library reuse, AsyncAPI source/recovery and valid fixture rendering, and mocked session refresh recovery.
- Three loader unit tests pass: deployment fingerprint selection, concurrent-load deduplication and retry after a failed load.
- Rust auth regression tests: tracked separately until the native test build completes.
- `cargo fmt --all --check` and `git diff --check` pass.
- Browser tests ran on this checkout's port 3100 against the isolated `lekton_ux_audit` metadata copy. Existing storage objects were read only. Real search used the local read-only proxy when enabled; no shared index configuration or production data changed. Port 3000 and the other worktree were not used.
- Selected schemas use the copied development snapshot. Its metadata version and embedded `info.version` can differ; this data discrepancy was not changed by the viewer integration.

Repeatable local command (set these corpus identifiers for the copied dataset):

```sh
LEKTON_READER_DOC=cocode/code-guidelines-app \
LEKTON_PRIVATE_DOC=cocode/code-guidelines-app \
LEKTON_OPENAPI_SCHEMA=http/assets-manager \
LEKTON_ASYNCAPI_SCHEMA=mqtt/directory \
npx playwright test --config playwright.ux.config.ts
npm run test:ui-unit
cargo fmt --all --check
```

The dedicated browser config starts no server and seeds no content. The default CI fixtures remain supported; the restricted-document check requires an explicitly supplied private document.

## Rendered evidence

Baseline OpenAPI light/390×844: [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6bot2r-4746a9e6.png).
After same schema, viewport and light theme: [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6ovgcz-5e0ada11.png).
After dark/390×844: [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6ovgmp-a99a83b8.png).
Baseline dark download control: [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6bo95x-170e812e.png).
The desktop baseline and narrow captures differ in viewport; they are not a pixel-identical pair.

At all four widths in both modes, page scroll width equals viewport width. No schema sidebar label extends beyond the navigation column. These are browser CSS viewport checks, not physical touch tests.

| Theme | Viewport | Page scroll width | Evidence |
|---|---|---|---|
| light | 1440×900 | 1440 | [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6onuzp-fc316382.png) |
| light | 1024×900 | 1024 | [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6onvi3-19b21ea1.png) |
| light | 768×900 | 768 | [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6onw0i-190a4b40.png) |
| light | 390×900 | 390 | [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6onwa6-ba10e6ce.png) |
| dark | 1440×900 | 1440 | [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6onwzs-6bc0dfa4.png) |
| dark | 1024×900 | 1024 | [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6onxih-7d7966c2.png) |
| dark | 768×900 | 768 | [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6ony7y-b575240a.png) |
| dark | 390×900 | 390 | [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6onyi6-9c8f251c.png) |
