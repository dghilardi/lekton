---
name: Lekton
description: "The existing default portal identity, extended to the public website."
colors:
  light-primary: "oklch(50%    0.19  64)"
  light-primary-content: "oklch(98%    0.006 85)"
  light-secondary: "oklch(52%    0.14  248)"
  light-secondary-content: "oklch(97%    0.006 248)"
  light-accent: "oklch(60%    0.19  30)"
  light-accent-content: "oklch(98%    0.004 30)"
  light-base-100: "oklch(98.5%  0.003 85)"
  light-base-200: "oklch(95%    0.005 80)"
  light-base-300: "oklch(90%    0.009 72)"
  light-base-content: "oklch(21%    0.008 258)"
  dark-primary: "oklch(79%    0.155 72)"
  dark-primary-content: "oklch(14%    0.015 72)"
  dark-secondary: "oklch(65%    0.13  200)"
  dark-secondary-content: "oklch(10%    0.01  200)"
  dark-accent: "oklch(70%    0.17  35)"
  dark-accent-content: "oklch(10%    0.01  35)"
  dark-base-100: "oklch(11%    0.011 241)"
  dark-base-200: "oklch(15%    0.013 241)"
  dark-base-300: "oklch(22%    0.016 241)"
  dark-base-content: "oklch(91%    0.006 248)"
typography:
  body:
    fontFamily: "\"Plus Jakarta Sans\", \"Geist\", ui-sans-serif, system-ui, sans-serif"
  document-body:
    fontFamily: "\"Plus Jakarta Sans\", \"Geist\", ui-sans-serif, system-ui, sans-serif"
    fontSize: "0.9375rem"
    lineHeight: 1.8
  website-display:
    fontFamily: "\"Plus Jakarta Sans\", \"Geist\", ui-sans-serif, system-ui, sans-serif"
    fontSize: "clamp(2rem, 3.5vw, 3.4rem)"
    fontWeight: 700
    lineHeight: 1.12
    letterSpacing: "-0.025em"
  website-headline:
    fontFamily: "\"Plus Jakarta Sans\", \"Geist\", ui-sans-serif, system-ui, sans-serif"
    fontSize: "1.75rem"
    fontWeight: 700
    lineHeight: 1.22
    letterSpacing: "-0.025em"
  menu-label:
    fontFamily: "\"Plus Jakarta Sans\", \"Geist\", ui-sans-serif, system-ui, sans-serif"
    fontSize: "0.64rem"
    fontWeight: 700
    letterSpacing: "0.09em"
  code:
    fontFamily: "\"Fira Code\", \"JetBrains Mono\", ui-monospace, monospace"
rounded:
  selector: "0.4rem"
  field: "0.35rem"
  box: "0.5rem"
  badge: "0.3rem"
spacing:
  xs: "0.25rem"
  sm: "0.5rem"
  md: "1rem"
  lg: "1.5rem"
  xl: "2rem"
components:
  website-button-primary:
    backgroundColor: "{colors.light-primary}"
    textColor: "{colors.light-primary-content}"
    rounded: "{rounded.field}"
    padding: "0.65rem 1.75rem"
  website-button-primary-dark:
    backgroundColor: "{colors.dark-primary}"
    textColor: "{colors.dark-primary-content}"
    rounded: "{rounded.field}"
    padding: "0.65rem 1.75rem"
  website-button-secondary:
    textColor: "{colors.light-base-content}"
    rounded: "{rounded.field}"
    padding: "0.65rem 1.75rem"
  website-icon-button:
    rounded: "{rounded.field}"
    padding: "0.5rem"
    width: "2.75rem"
    height: "2.75rem"
  app-menu-item:
    rounded: "{rounded.selector}"
    padding: "0.45rem 0.625rem"
  website-cli-chip:
    backgroundColor: "{colors.light-base-200}"
    textColor: "{colors.light-base-content}"
    typography: "{typography.code}"
    rounded: "{rounded.field}"
    padding: "0.45rem 0.65rem"
  website-production-note:
    backgroundColor: "{colors.light-base-200}"
    textColor: "{colors.light-base-content}"
    rounded: "{rounded.box}"
    padding: "1.5rem 1.75rem"
  website-code-block:
    typography: "{typography.code}"
    rounded: "{rounded.field}"
    padding: "0.75rem 1rem"
---
# Design System: Lekton

## Overview

**Creative North Star: "Existing Lekton default identity"**

Lekton uses the existing portal identity: warm light surfaces, cool blue-slate dark surfaces, amber actions, and readable developer-oriented typography. The public website extends this identity with the minimal presentation approved on 2026-10-03. Its factual content and illustrative interface keep the product understandable without inflated marketing claims.

`style/tailwind.css` is the implementation authority for default colors, font stacks, spacing, and radii. `website/theme.mjs` extracts the shared font, surface, primary, and radius tokens at build time; this document records their current values rather than creating an independent palette. Application installations can override their styles through `public/custom.css`.

**Key Characteristics:**

- Shared application and website identity.
- Warm light / cool dark surfaces with amber emphasis.
- Compact controls, readable code, and small corners.

## Colors

### Primary

Amber identifies principal actions, links, the stacked outline mark, and active navigation. Light uses the richer amber; dark uses the brighter amber with dark text on filled actions. The paired `primary-content` token owns contrast on filled controls.

### Secondary

The application retains slate-indigo in light and teal-blue in dark for secondary roles. These are incumbent tokens, not additional website accents.

### Tertiary

The application’s accent pair is warm terracotta in light and copper in dark. The public website currently relies on primary and neutral roles.

### Neutral

`base-100` is the page surface, `base-200` provides tonal grouping, `base-300` supplies borders and dividers, and `base-content` supplies text. Website muted text blends content with the current page surface using `color-mix(in oklch, …)`; it does not introduce separate fixed grays. Existing application success, warning, error, and info tokens retain their semantic meaning in the source stylesheet.

The frontmatter records both default modes in their source OKLCH notation. Component frontmatter shows light defaults and an explicit dark primary variant; actual implementations bind to theme-sensitive CSS variables.

## Typography

Plus Jakarta Sans is the shared interface and heading family; Fira Code distinguishes commands and code. Preserve the existing fallback stacks. Application body text enables `cv03`, `cv04`, and `cv11`; code enables contextual alternates and ligatures.

Application Markdown uses the `document-body` role, with headings in the same sans family and tighter tracking. Menu section labels are small, bold, spaced uppercase; interface badges follow a similar compact label treatment.

The public website uses `website-display` for its opening heading and `website-headline` for section headings. Its body has a line height of (1.6), supporting copy often uses (0.875rem), and technical blocks use (0.8rem) with generous line height. These sizes describe this surface, not the application’s global heading scale. Website prose is constrained to roughly (65–75ch) where the implementation provides a reading measure.

## Layout

The application’s established layout tokens are a sidebar width of (16rem), content maximum of (72rem), and header height of (3.75rem). Its shared spacing steps are captured in the frontmatter.

**Public website:** the container caps at (83.75rem) with fluid side margins. Desktop opening content and portal preview use a (44% / 53%) split with a (3%) gap. Capability and setup sections use equal columns; thin separators organize the page. At (800px) and below, major sections stack in source order. At (520px) and below, navigation wraps onto a second header row and the illustrative portal becomes denser. Technical blocks scroll horizontally when needed. These are website-specific compositions.

## Elevation & Depth

The website is flat: borders, spacing, and tonal surfaces provide grouping without box shadows. The application retains its existing hover shadows on cards, dropdown shadows, subtle code-block shadows, and dark dot-grid background. The website’s restraint does not remove those incumbent treatments. Exact shadow and transition values are recorded in the sidecar.

## Shapes

Controls use the established field radius, selector items use the selector radius, and framed or grouped surfaces use the box radius. Badges have their own compact radius. Thin one-pixel borders distinguish adjoining surfaces. The stacked outline logo and simple stroked icons continue the product’s existing identity.

## Components

- **Primary and secondary actions:** small corners, compact semibold labels; primary actions pair theme-sensitive amber with its content color. Website secondary actions use an outline and a subtle tonal hover. Keyboard focus is an amber outline.
- **Theme control:** a compact icon button cycles system → light → dark → system. Both surfaces use `lekton-theme` in local storage; system mode clears the saved override and follows the OS preference. The website switches its embedded preview together with the page.
- **Navigation:** website links use plain text and amber hover; locale links remain ordinary URLs. Application menu items use compact padding and an amber-tinted active background, with `aria-current="page"` identifying selection.
- **Fields:** application inputs, textareas, and selects retain the shared sans family and short border/shadow transitions. The website’s preview search is a static illustrative label, not an interactive field.
- **Chips and badges:** the website’s CLI chip has a tonal surface, outline, and mono label. Application badges retain their compact uppercase typography and semantic variants.
- **Containers:** the website portal frame is outlined; its production note is a quiet tonal group. Application cards retain their existing hover depth.
- **Technical blocks:** website code has a mono language header, optional copy action, visible status feedback, and scrollable content. Amber highlights YAML keys in the illustrative preview.
- **Disclosure rows:** native `details` and `summary` reveal capability or sync content; a small chevron rotates on expansion. Content remains accessible without JavaScript. Website transitions are disabled for reduced motion.

## Do's and Don'ts

### Do:

- **Do** inherit default theme tokens from `style/tailwind.css` and regenerate the website theme at build time.
- **Do** use the same Plus Jakarta Sans and Fira Code font stacks across application and public website.
- **Do** retain visible keyboard focus, responsive reading order, and reduced-motion support on the website.
- **Do** distinguish the website’s illustrative portal preview from the running application.

### Don't:

- **Don’t** introduce an independently maintained website palette.
- **Don’t** restore the rejected loud blue marketing direction on the public website.
- **Don’t** apply the website’s split hero or flat depth treatment as a new rule for application screens.
