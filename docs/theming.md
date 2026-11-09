# Theming

Lekton can be re-skinned at runtime, without rebuilding, by setting a small set
of CSS custom properties. A theme is a stylesheet that **only assigns
variables**: the components read them, so a theme does not need to know the
markup, chase selectors, or use `!important`.

Where to put a theme:

- **Admin → Custom CSS** — stored in MongoDB and injected after the core
  stylesheet on every page. Changes apply on the next page load.
- **`public/custom.css`** — served as `/custom.css` and loaded after the core
  stylesheet. Mount your own file over it (for example as a Docker volume).

Both are loaded after the core stylesheet, so a variable you assign there wins
over the default.

## How to assign variables

The page sets `data-theme="light"` or `data-theme="dark"` on `<html>`. Assign
variables on `:root` to apply them to both modes, or on `[data-theme="light"]`
/ `[data-theme="dark"]` for one mode:

```css
:root {
  --lekton-font-family: "Inter", sans-serif;     /* both modes */
}

[data-theme="light"] {
  --lekton-sidebar-bg: #0f172a;                  /* light mode only */
}

[data-theme="dark"] {
  --lekton-sidebar-bg: #020617;                  /* dark mode only */
}
```

Assign variables on `:root` / `[data-theme]` rather than on inner elements:
several defaults are derived from other variables (for example the muted
sidebar text is 70 % of `--lekton-sidebar-text`), and the derivation happens
where the variable is assigned.

Every variable has a default derived from the DaisyUI palette, so an empty
custom stylesheet gives the stock look, and a theme only needs the variables it
wants to change.

## Variables

### Palette and shape (DaisyUI)

The DaisyUI 5 variables define the base palette used by every component:
`--color-base-100`, `--color-base-200`, `--color-base-300`,
`--color-base-content`, `--color-primary` / `--color-primary-content`, and the
same pairs for `secondary`, `accent`, `neutral`, `info`, `success`, `warning`
and `error`. Corner radii are `--radius-box` (cards, modals), `--radius-field`
(inputs, buttons) and `--radius-selector` (checkboxes, toggles, badges).

Changing the palette is often enough: the header, sidebar, content and code
blocks default to it.

### Typography and layout

| Variable | Default | Used for |
|---|---|---|
| `--lekton-font-family` | Plus Jakarta Sans | UI and body text |
| `--lekton-heading-font` | `--lekton-font-family` | Headings |
| `--lekton-mono-font` | Fira Code | Code, `kbd` |
| `--lekton-sidebar-width` | `16rem` | Sidebar width |
| `--lekton-content-max-width` | `72rem` | Page content width |
| `--lekton-header-height` | `3.75rem` | Anchor scroll offset |

Load web fonts with an `@import` at the top of the theme.

### Header

| Variable | Default | Used for |
|---|---|---|
| `--lekton-header-bg` | base-100 at 80 % | Bar background (blurred) |
| `--lekton-header-text` | page text | Bar text; navigation links use 80 % of it |
| `--lekton-header-hover-bg` | base-200 at 50 % | Navigation link hover |
| `--lekton-header-border-color` | base-200 (dark: base-300 at 70 %) | Bottom border |
| `--lekton-header-border-width` | `1px` | Bottom border |
| `--lekton-header-shadow` | small shadow | Bar shadow |

Menus that open from the header keep the page colours.

### Sidebar

The sidebar holds the documentation tree, the chat history and the admin menu.

| Variable | Default | Used for |
|---|---|---|
| `--lekton-sidebar-bg` | base-200 | Background |
| `--lekton-sidebar-text` | page text | Text; section titles use 80 %, chat history 60 % of it |
| `--lekton-sidebar-text-muted` | 80 % of the sidebar text | Document links, chat history |
| `--lekton-sidebar-heading` | 65 % of the sidebar text | Group headings ("Navigation", "AI Chat") |
| `--lekton-sidebar-accent` | primary | Link text on hover |
| `--lekton-sidebar-hover-bg` | base-300 at 60 % | Item background on hover |
| `--lekton-sidebar-active-bg` | primary at 10 % | Current page / chat |
| `--lekton-sidebar-active-text` | primary | Current page / chat |
| `--lekton-sidebar-border` | base-300 | Right border |
| `--lekton-sidebar-tree-line` | base-300 | Guide line of nested levels |

A dark sidebar on a light page needs `--lekton-sidebar-bg` and
`--lekton-sidebar-text`; everything else follows.

### Page content

| Variable | Default | Used for |
|---|---|---|
| `--lekton-content-text` | page text at 85 % | Body text of documents |
| `--lekton-heading-color` | page text | Headings and bold text |
| `--lekton-link-color` | primary | Links |
| `--lekton-link-decoration` | `underline` | Link underline (`none` underlines on hover only) |
| `--lekton-table-header-bg` | `transparent` | Table header background |
| `--lekton-table-header-text` | page text | Table header text |
| `--lekton-table-border` | base-300 | Table rules |

### Code

| Variable | Default | Used for |
|---|---|---|
| `--lekton-code-bg` | base-200 | Code block background |
| `--lekton-code-text` | page text | Code block text |
| `--lekton-code-border` | base-300 | Code block border |
| `--lekton-code-radius` | `0.6rem` | Code block corners |
| `--lekton-code-toolbar-text` | `--lekton-code-text` | Language badge and copy button |
| `--lekton-code-copy-border` | `--lekton-code-border` | Copy button border |
| `--lekton-code-copy-hover-bg` | `--lekton-code-border` | Copy button hover |
| `--lekton-code-scheme` | `light` (dark mode: `dark`) | Syntax palette: `light` or `dark` |
| `--lekton-inline-code-bg` | base-200 | Inline `code` background |
| `--lekton-inline-code-text` | page text at 90 % | Inline `code` text |
| `--lekton-inline-code-border` | base-300 | Inline `code` border |

`--lekton-code-scheme` sets the code block's `color-scheme`, which selects the
light or dark syntax palette. Set it to `dark` when code blocks get a dark
background in light mode, so keywords and strings stay readable. Individual
syntax colours can still be overridden with `--lekton-code-keyword`,
`-string`, `-comment`, `-constant`, `-function`, `-type`, `-variable`, `-key`,
`-section`, `-inserted`, `-deleted` and `-invalid`.

## Example: an "Acme" theme

A navy sidebar and dark code blocks on a light page, with a teal accent:

```css
@import url("https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap");

:root {
  --lekton-font-family: "Inter", sans-serif;
  --radius-box: 0.75rem;
  --radius-field: 0.5rem;
  --lekton-header-border-width: 2px;
  --lekton-header-border-color: #14b8a6;
  --lekton-link-decoration: none;

  /* Dark code blocks in both modes */
  --lekton-code-scheme: dark;
  --lekton-code-bg: #0b1220;
  --lekton-code-text: #e2e8f0;
  --lekton-code-border: #1e293b;
}

[data-theme="light"] {
  --color-primary: #0d9488;
  --color-primary-content: #ffffff;

  --lekton-sidebar-bg: #0f172a;
  --lekton-sidebar-text: #e2e8f0;
  --lekton-sidebar-heading: #2dd4bf;
  --lekton-sidebar-accent: #2dd4bf;
  --lekton-sidebar-hover-bg: rgb(45 212 191 / 0.12);
  --lekton-sidebar-active-bg: rgb(45 212 191 / 0.16);
  --lekton-sidebar-active-text: #2dd4bf;
  --lekton-sidebar-border: #1e293b;
  --lekton-sidebar-tree-line: rgb(226 232 240 / 0.15);

  --lekton-table-header-bg: #0f172a;
  --lekton-table-header-text: #ffffff;
}

[data-theme="dark"] {
  --color-primary: #2dd4bf;
  --color-primary-content: #042f2e;
  --lekton-sidebar-bg: #020617;
}
```

## Stable hooks

When a variable does not cover what you need, use these attributes rather than
utility classes, which change between releases:

| Hook | Element |
|---|---|
| `[data-lekton-header]` | Top bar |
| `[data-lekton-sidebar]` | Sidebar panel |
| `[data-lekton-markdown]` | Rendered Markdown (documents, chat answers) |
| `[data-lekton-chat-session]` | A chat history entry (`aria-current` when open) |
| `[data-lekton-chat-status]` | Notice under an unfinished chat answer (`interrupted` / `error`) |
| `[data-lekton-chat-input]` | Chat message box |
| `.code-copy-btn`, `.code-lang-badge` | Code block toolbar |
| `.lekton-logo-light`, `.lekton-logo-dark` | Logo variants |

For example, a coloured rule under second-level headings:

```css
[data-lekton-markdown] h2 { border-bottom: 2px solid var(--color-primary); }
```

## Upgrading an older custom stylesheet

Stylesheets written against earlier releases keep working: the header and
sidebar keep their previous classes, and rules with higher specificity or
`!important` still win over the token defaults. Moving to the variables makes a
theme much shorter and fixes elements that selector-based themes tend to miss —
for example the chat history entries (buttons, not links) in a dark sidebar, or
the copy button on dark code blocks. Typical translations:

| Older rule | Variable |
|---|---|
| `.drawer-side .menu { background-color: … }` | `--lekton-sidebar-bg` |
| `.drawer-side .menu a { color: … }` | `--lekton-sidebar-text`, `--lekton-sidebar-text-muted` |
| `.menu a[aria-current="page"] { … }` | `--lekton-sidebar-active-bg`, `--lekton-sidebar-active-text` |
| `header.fixed { … }` | `--lekton-header-bg`, `--lekton-header-text`, `--lekton-header-border-*` |
| `.prose pre { background-color: … }` | `--lekton-code-bg`, `--lekton-code-text`, `--lekton-code-scheme` |
| `.prose a { color: … }` | `--lekton-link-color`, `--lekton-link-decoration` |
| `.prose thead { … }` | `--lekton-table-header-bg`, `--lekton-table-header-text` |
