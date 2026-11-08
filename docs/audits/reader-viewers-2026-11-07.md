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
- Within the portal's content width the AsyncAPI viewer always collapses its navigation behind a floating `div` toggle with no role, focus or keyboard handling, so channel links were unreachable from the keyboard. The adapter now gives the toggle button semantics, Enter/Space activation and `aria-expanded`, without replacing the library's handler.
- CI (release build) showed intermittent server panics (`reactive_graph` `arc_memo.rs:334`, `unwrap()` on `None`) that aborted unrelated page loads. A debug build reproduced them under concurrent schema-page requests: at the end of an SSR response, releasing the schema resource woke the content resource, whose source then read already-disposed router params. That source now reads only reference-counted signals and fallible reads. 1200 concurrent anonymous and authenticated requests then produced no panic, against 2–3 panics per 240–300 requests before.
- That fix did not cover the release-build panic seen on the home page. On a local stack seeded like CI (isolated MongoDB, Meilisearch and MinIO containers), concurrent requests reproduced it: 9 panics in 1200 requests. Each Suspense boundary resolves in its own task on the multi-threaded runtime, and two boundaries computing the same uncomputed memo (`ActiveReleasePins`) at once can leave one reader with no value (`arc_memo.rs:334`). The remaining panics were reads of values already disposed after such an abort. The layout now computes its shared URL-derived memos before any boundary renders, and the schema tree reads the router location fallibly. Afterwards: 0 panics in 2400 requests, and the complete CI browser suite passed locally against that stack without flaky retries.
- Schema version selection is derived from the resolved metadata and the per-schema user selection, not mutated during render. This prevents a redundant content read during hydration.

## Authentication and delivery boundaries

Existing server functions still enforce access levels. No process-global reader-data cache was introduced. Reader HTML is `private, no-store`. Client navigations use the shared auth retry orchestrator. A failed SSR session is recovered through auth bootstrap, then only failed unauthorized reads are retried. A deterministic browser test simulates successful refresh; it does not claim to test an external OAuth provider or actual refresh-token rotation.

The initial document is readable with JavaScript disabled. Interactive viewers progressively enhance an SSR-readable source. Successful initial reads do not need WASM to expose the content. The raw HTML can be larger than a shell-only response; fewer API round trips do not establish a bandwidth or latency percentage improvement. Production compression, cold-network timing, heap profiling and physical-device/screen-reader checks remain outside the measured coverage.

## Bundle budget

Release frontend builds of `75a3cc0` and of this branch used the same cargo-leptos command, one Cargo job and the same feature set. Sizes are bytes on disk; gzip -9 sizes are indicative only, since production compression was not measured.

| Asset | `75a3cc0` | This branch | Delta |
|---|---|---|---|
| `lekton.wasm` | 5192965 (gzip 1883889) | 5349797 (gzip 1936287) | +156832 |
| `lekton.js` | 44699 | 45269 | +570 |
| `lekton.css` | 273196 | 278007 | +4811 |
| Tiptap bundle on reader pages | 384787 (gzip 115227) | 0, loaded only in the editor | −384787 |
| Reader total | 5895647 | 5673073 | −222574 |

The WASM grows by about 3%, mostly from the serialization code that hydrates server-resolved reader data. Readers still download less overall, because the editor bundle is no longer eager. The CI WASM threshold therefore moves from 5120 KiB to 5376 KiB, leaving about 150 KiB of headroom above this build rather than removing the guard.

## Verification

- Both cargo-leptos frontend and server builds passed with one Cargo job, dev debug information disabled and incremental compilation disabled.
- 19 browser checks passed: preceding portal keyboard/theme regressions plus SSR HTML, no-JS reading, anonymous denial of a private document, shared schema reads, lazy editor loading, OpenAPI geometry/theme/keyboard navigation/library reuse, AsyncAPI source/recovery and valid fixture rendering, and mocked session refresh recovery.
- Follow-up on 8 November: 35 browser checks passed on a server restarted from the final debug build. Added coverage: schema version switching (one content read, one viewer library), viewer module import failure, no-JS viewer notice, JSON Schema formatting, expired-session sign-in guidance without JavaScript, bounded client retry (recovered, still unauthorized, refresh rejected, unrelated error), keyboard navigation of AsyncAPI channels, and sidebar geometry at 1440/1024/768/390 px in both themes.
- `cargo clippy --features ssr --lib -- -D warnings` and `cargo clippy --features hydrate --lib -- -D warnings` pass.
- Four loader unit tests pass: deployment fingerprint selection, concurrent-load deduplication, retry after a failed load, and the editor loader sharing the versioned viewer module.
- Six Rust auth-refresh regression tests pass. The first native build exhausted the VM's disk; after space was available, the serial rebuild and all six tests completed successfully.
- `cargo fmt --all --check` and `git diff --check` pass.
- Browser tests ran on this checkout's port 3100 against the isolated `lekton_ux_audit` metadata copy. Existing storage objects were read only. Real search used the local read-only proxy when enabled; no shared index configuration or production data changed. Port 3000 and the other worktree were not used.
- Selected schemas use the copied development snapshot. Its metadata version and embedded `info.version` can differ; this data discrepancy was not changed by the viewer integration.

Repeatable local command (set these corpus identifiers for the copied dataset):

```sh
LEKTON_READER_DOC=cocode/code-guidelines-app \
LEKTON_PRIVATE_DOC=cocode/code-guidelines-app \
LEKTON_OPENAPI_SCHEMA=http/assets-manager \
LEKTON_ASYNCAPI_SCHEMA=mqtt/directory \
LEKTON_MULTI_VERSION_SCHEMA=http/remotecommand \
npx playwright test --config playwright.ux.config.ts
npm run test:ui-unit
cargo fmt --all --check
```

The dedicated browser config starts no server and seeds no content. The default CI fixtures remain supported; the restricted-document check requires an explicitly supplied private document.

## Rendered evidence

Baseline OpenAPI dark/390×844: [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6bot2r-4746a9e6.png).
After the same schema, viewport and dark theme: [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6ovgmp-a99a83b8.png).
Additional light/390×844 check: [PNG](/home/davide.ghilardi/.t3/userdata/browser-artifacts/browser-screenshot-localhost-mw6ovgcz-5e0ada11.png).
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
