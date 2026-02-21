# Lekton Theme Gallery

This directory contains pre-built themes that demonstrate Lekton's runtime theming capabilities. Each theme is a complete CSS file that can be copied to `public/custom.css` to instantly transform the look and feel of your documentation portal.

## 🎨 Available Themes

### 🌙 GitHub Dark (`github-dark.css`)

A developer-friendly dark theme inspired by GitHub's dark mode interface.

**Best for:**
- Development teams familiar with GitHub
- Late-night documentation sessions
- Code-heavy documentation

**Color Palette:**
- Background: `#0d1117` (deep dark blue)
- Text: `#c9d1d9` (light gray)
- Primary: `#1f6feb` (GitHub blue)
- Accent: `#8b68ff` (purple)
- Success: `#238636` (green)

**Typography:**
- Font: System UI stack (San Francisco, Segoe UI, etc.)
- Code: Monospace with yellow syntax highlighting

**Special Features:**
- Subtle hover effects on links and menu items
- GitHub-style code block borders
- Compact, efficient spacing

---

### ❄️ Nord (`nord.css`)

An arctic, north-bluish color palette that's easy on the eyes and beautiful to look at.

**Best for:**
- Teams prioritizing aesthetics
- Long documentation reading sessions
- Modern, minimalist brands

**Color Palette:**
- Background: `#2e3440` (Polar Night)
- Text: `#d8dee9` (Snow Storm)
- Primary: `#88c0d0` (Frost - light blue)
- Secondary: `#8fbcbb` (Frost - teal)
- Accent: `#bf616a` (Aurora - red)

**Typography:**
- Font: Inter (sans-serif)
- Clean, modern typeface
- Comfortable line height (1.8)

**Special Features:**
- Smooth transitions and animations
- Hover effects with subtle color shifts
- Uppercase menu titles with letter spacing
- Enhanced blockquotes with background tint

---

### ☀️ Solarized Light (`solarized-light.css`)

A precision-crafted light theme with scientifically selected colors for maximum readability.

**Best for:**
- Traditional documentation portals
- Print-friendly content
- Academic or professional environments
- Daytime reading

**Color Palette:**
- Background: `#fdf6e3` (warm cream)
- Text: `#657b83` (gray-blue)
- Primary: `#268bd2` (blue)
- Accent: `#859900` (green)
- Warning: `#b58900` (yellow)

**Typography:**
- Font: Georgia (serif) for a classic feel
- Larger base size (1.125rem)
- Justified text with hyphenation
- High line height (1.8)

**Special Features:**
- Decorative blockquotes with quotation marks
- Striped table rows
- Elegant shadows and borders
- Smooth hover transitions with transforms

---

## 🚀 How to Use

### Development Mode

Copy the theme to `public/custom.css`:

```bash
# From the project root
cp examples/themes/github-dark.css public/custom.css
```

Then reload your browser - no compilation needed!

### Docker / Production

Mount the theme as a volume in your `docker-compose.yml`:

```yaml
services:
  lekton:
    # ... other config
    volumes:
      - ./examples/themes/nord.css:/app/public/custom.css
```

Or with `docker run`:

```bash
docker run \
  -v ./examples/themes/solarized-light.css:/app/public/custom.css \
  lekton
```

---

## 🛠️ Customizing Themes

Each theme is fully customizable. Start with a base theme and modify it:

### 1. Copy the theme
```bash
cp examples/themes/nord.css my-custom-theme.css
```

### 2. Edit the colors

All themes use DaisyUI's color system. Find the color definitions (usually at the top):

```css
html[data-theme="light"] {
  --p: 193 43% 67%;    /* Primary color */
  --s: 179 25% 65%;    /* Secondary color */
  --a: 14 51% 63%;     /* Accent color */
  /* ... more colors ... */
}
```

### 3. Adjust typography

```css
:root {
  --lekton-font-family: "Your Font", sans-serif;
  --lekton-sidebar-width: 18rem;
  --lekton-content-max-width: 80rem;
}
```

### 4. Fine-tune components

Each theme includes custom styling for:
- Prose (markdown content)
- Code blocks
- Tables
- Links
- Buttons
- Navigation
- Cards

Simply modify the CSS rules to match your preferences.

---

## 🎨 Color Format

DaisyUI uses OKLCH color space for better color consistency. The format is:

```css
--p: HUE SATURATION LIGHTNESS;
```

Examples:
- `--p: 212 92% 45%` → Blue
- `--a: 137 55% 40%` → Green
- `--er: 358 77% 47%` → Red

**Tip:** Use online tools like [OKLCH Color Picker](https://oklch.com/) to find the perfect colors.

---

## 📚 Theme Anatomy

Each theme typically includes:

1. **Base Color Definitions** - DaisyUI theme variables
2. **Lekton Design Tokens** - Font, spacing, layout
3. **Prose Styling** - Markdown content appearance
4. **Component Overrides** - Buttons, cards, navigation
5. **Special Effects** - Animations, shadows, transitions

---

## 🌈 Creating a Brand Theme

To create a theme matching your company's brand:

1. Start with the theme that's closest to your desired style
2. Replace color values with your brand colors
3. Update the font to your brand typeface
4. Adjust spacing to match your design system
5. Add your logo's color to the primary variable

Example brand theme:

```css
/* Acme Corp Theme */
html[data-theme="light"] {
  --p: 15 80% 50%;      /* Acme Orange #e84118 */
  --s: 220 13% 18%;     /* Acme Dark Gray */
  --a: 195 100% 40%;    /* Acme Blue */
}

:root {
  --lekton-font-family: "Acme Sans", "Open Sans", sans-serif;
}
```

---

## 🤝 Contributing Themes

Have a beautiful theme you'd like to share? We welcome contributions!

1. Create your theme in this directory
2. Add a section in this README describing it
3. Submit a pull request

Please ensure your theme:
- Uses semantic color names
- Includes comments explaining the color choices
- Provides both light and dark variants (if applicable)
- Maintains good contrast ratios (WCAG AA)

---

## 📖 Resources

- [DaisyUI Themes Documentation](https://daisyui.com/docs/themes/)
- [OKLCH Color Picker](https://oklch.com/)
- [Tailwind CSS Documentation](https://tailwindcss.com/)
- [Web Content Accessibility Guidelines (WCAG)](https://www.w3.org/WAI/WCAG21/quickref/)

---

## 📝 License

All themes in this directory are MIT licensed, same as Lekton itself.
