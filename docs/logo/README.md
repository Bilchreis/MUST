# MUST logo

The word MUST under a bar whose end is cut at a slant. The bar and its cut are taken
from the [SECoP](https://github.com/SampleEnvironment/SECoP) logo, whose S runs on as a
bar over the rest of the word.

## Files

| File | Use |
|---|---|
| `must-logo.svg` | the logo on light backgrounds |
| `must-logo-dark.svg` | the logo on dark backgrounds |
| `must-logo-black.svg`, `must-logo-white.svg` | one color, for print, stamps and engraving |
| `must-icon.svg`, `must-icon-dark.svg` | the M under the bar, without a background |
| `must-icon-tile.svg` | the icon on a dark rounded tile: avatars, app icons, the favicon |
| `social-preview.svg` | the logo on the dark background, 1280×640, for link previews |
| `favicon.ico` | the tile icon at 16, 32 and 48 px |
| `png/must-logo.png`, `png/must-logo-dark.png` | the logo, 1200 px wide, transparent |
| `png/must-icon-<size>.png` | the tile icon at 16 to 512 px |
| `png/must-icon-transparent-512.png` | the icon without a background |
| `png/social-preview.png` | the link preview, 1280×640 |

The SVG files are the originals and hold the letters as outlines, so no font is needed
to show them. The PNG files are exported from them, for example with
`inkscape must-logo.svg -w 1200 -o png/must-logo.png`.

## Colors and type

| | |
|---|---|
| Orange (the bar) | `#E8590C` |
| Dark (letters on light, backgrounds) | `#101820` |
| Light (letters on dark) | `#F3F4F1` |

The letters are [JetBrains Mono](https://github.com/JetBrains/JetBrainsMono) ExtraBold.

## Using it

- Keep clear space of at least the bar's thickness around the logo; the SVG files
  already include about half of that.
- Use the icon instead of the logo below about 80 px width.
- Do not stretch the logo, recolor the letters or move the bar.

## Where it is used

- the [README](../../README.md) and the first page of the guide (`../introduction.md`,
  switched with the theme by `../theme/logo.css`)
- the guide's favicon: `../theme/favicon.svg` and `../theme/favicon.png` are copies of
  `must-icon-tile.svg` and `png/must-icon-32.png`
- the empty overview of the status TUI, drawn with block characters (`logo` in
  `crates/must-tui/src/ui.rs`)
