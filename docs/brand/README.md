# Krasnokamen brand

<p>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="krasnokamen-logo-on-dark.svg">
    <img src="krasnokamen-logo-on-light.svg" alt="Krasnokamen" width="420">
  </picture>
</p>

Krasnokamen is an engine for large Minecraft-compatible simulations. This page
holds its logo files, colours, type and naming rules. Every asset here is
original art drawn for the project: no Mojang textures, no Minecraft imagery.

## Name

- Write **Krasnokamen**: one word, capital K, the rest lower case. Not
  "KrasnoKamen", "Krasno Kamen", "Krasnokamen Engine" or "KK".
- Meaning: from Russian «красный камень», "red stone". The stone is a rock,
  not Minecraft's redstone, and the art never shows redstone dust, torches or
  circuits.
- English pronunciation: **kras-NO-ka-men**.
- Components keep the name in front: Krasnokamen Client, Krasnokamen Loader,
  Krasnokamen Default Plugins.
- Code uses the lower-case id `krasnokamen`: the binary, crates and packages
  (`krasnokamen-*`), identifiers (`krasnokamen_*`), namespaces and channels
  (`krasnokamen:*`). Environment variables use `KRASNOKAMEN_*`. Upper case
  KRASNOKAMEN appears only in the wordmark and in environment variables.
- Describe compatibility, never affiliation: "Minecraft-compatible", "for
  Minecraft: Java Edition". Keep the notice that the project is not approved by
  or affiliated with Mojang Studios or Microsoft.

## Formerly Solaris

Until 2026-11-12 the project was called **Solaris**. It was renamed because
SOLARIS is a live US trademark of two companies in the same class of goods
(class 9, software):

- Oracle America, Inc.: US Reg. No. 1863585, computer operating system and
  utility software ([USPTO record](https://tsdr.uspto.gov/statusview/sn74452720));
- Atari Interactive, Inc.: US Reg. No. 7213105, recorded computer and video
  game software ([USPTO record](https://tsdr.uspto.gov/statusview/rn7213105)).

Old release notes, decision records and the bug ledger keep the name Solaris
where they describe the past. Write "Krasnokamen (formerly Solaris)" once when
a reader may know the old name; otherwise use Krasnokamen alone.

## Files

| File | Use |
| :-- | :-- |
| [`krasnokamen-logo-on-light.svg`](krasnokamen-logo-on-light.svg) | Primary logo (mark and wordmark) for light backgrounds |
| [`krasnokamen-logo-on-dark.svg`](krasnokamen-logo-on-dark.svg) | Primary logo for dark backgrounds: light wordmark, lower stone facets one step lighter |
| [`krasnokamen-wordmark-on-light.svg`](krasnokamen-wordmark-on-light.svg), [`krasnokamen-wordmark-on-dark.svg`](krasnokamen-wordmark-on-dark.svg) | Wordmark alone, where the mark already appears nearby |
| [`krasnokamen-mark-on-light.svg`](krasnokamen-mark-on-light.svg), [`krasnokamen-mark-on-dark.svg`](krasnokamen-mark-on-dark.svg) | The faceted red stone alone, transparent |
| [`krasnokamen-icon.svg`](krasnokamen-icon.svg) and `krasnokamen-icon-{16,32,64,256,512}.png` | Square app, launcher and favicon icon: the stone on a Stone 900 tile |

The wordmark is drawn as outlined SVG paths, so no font is needed to show it.
Never retype KRASNOKAMEN in a font to imitate it; use the files.

In a GitHub README, let the reader's theme pick the variant:

```html
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/kaiserproger/krasnokamen/main/docs/brand/krasnokamen-logo-on-dark.svg">
  <img src="https://raw.githubusercontent.com/kaiserproger/krasnokamen/main/docs/brand/krasnokamen-logo-on-light.svg" alt="Krasnokamen" width="480">
</picture>
```

## Mark and wordmark

The mark is a cut stone lit from the top left: eight flat facets in the red
ramp below, lightest on the top facet, darkest at the lower right. The wordmark
is a set of chiselled capitals: square stems, and every curve cut into straight
45° facets (the O is an octagon), echoing the stone.

- Clear space: keep free space of at least half the mark's height on every side
  of the logo.
- Minimum size: the logo at 160 px wide; the mark alone at 24 px. Below 32 px use
  the icon PNGs, which are rendered for those sizes.
- Do not recolour, outline, rotate, stretch, shadow or animate the mark or the
  wordmark, rearrange the lockup, place the logo on a busy picture, or combine
  it with Minecraft or Mojang logos and textures.

## Colour

Brand red is **`#C63A2E`**. The red ramp exists to shade the stone's facets;
use brand red alone for accents, links and highlights. Stone greys carry text
and surfaces.

| Name | Hex | Use |
| :-- | :-- | :-- |
| Red 100 | `#F7907A` | Top facet |
| Red 200 | `#EC6852` | Upper-left facet; red text on Stone 900 |
| Red 300 | `#DB4A3A` | Left facet |
| **Red 500 (brand red)** | **`#C63A2E`** | Front facet, accents, badges |
| Red 600 | `#B33229` | Lower-left facet |
| Red 700 | `#962827` | Right facet |
| Red 800 | `#7A1E21` | Bottom facet |
| Red 900 | `#5E161A` | Lower-right facet |
| Stone 50 | `#F5F3F1` | Wordmark and text on dark |
| Stone 100 | `#E9E6E3` | Light surfaces, panels |
| Stone 300 | `#C4BFBA` | Secondary text on dark, rules on light |
| Stone 500 | `#8C8681` | Muted text, disabled controls |
| Stone 700 | `#4E4A46` | Secondary text on light, rules on dark |
| Stone 800 | `#2F2C2A` | Raised dark surfaces |
| Stone 900 | `#1D1B1A` | Wordmark on light, dark backgrounds, icon tile |

On dark backgrounds (the `-on-dark` files, the icon and the social preview) the
right and lower facets move one step lighter, Red 500 to Red 800, so the stone
keeps its outline.

## Type

- Wordmark: custom drawn; only from the SVG files.
- Running text and headings: a neutral grotesque sans serif. Use Inter where
  you control the font, otherwise the system UI stack
  (`system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif`).
  Headings semibold, sentence case; the product name in running text stays
  "Krasnokamen", never all capitals.
- Code, commands and identifiers: a monospace face (JetBrains Mono or the
  system monospace).
- The social preview sets its tagline and release line in Nimbus Sans, a
  Helvetica-metric grotesque.
