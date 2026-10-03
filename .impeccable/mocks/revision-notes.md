# Public website composition revision

The original blue, condensed-display compositions were rejected by the user as too forceful and too close to marketing. They have no approval.

The revised direction is minimal and modern, with a single visual identity shared by landing page and portal. The current default theme is the starting point: Plus Jakarta Sans, Fira Code for code, the layered outline logo, amber actions, and the existing light and dark surfaces from `style/tailwind.css`.

The product and audience remain developers and platform teams evaluating and trying Lekton. English and Italian are both required.

`minimal-light.png` and `minimal-dark.png` show matching compositions in the existing palettes. The user approved both with “Ok, procedi” on 2026-10-03. The interface content is illustrative. The implementation uses semantic HTML for the explicitly labelled illustration, verified bilingual content and theme tokens extracted directly from the application rather than independent palette definitions.

The application was not running locally and there were no committed screenshot fixtures. The default-theme reference at `.impeccable/reference/theme.html` uses tokens copied directly from the current stylesheet. T3 preview loaded it and computed the palette values, but its screenshot tool repeatedly failed; no application screenshot has been claimed or used as evidence.

The approved restrained direction is implemented in `website/`. Application theme values take precedence over approximate colors in the generated mockups. No application theme changes have been made.
