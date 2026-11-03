# 0008. Fonts: runtime outlines and bitmap packs

- Status: Accepted; the default UI font is Source Sans 3 since 2026-10-19,
  not Atkinson Hyperlegible Next; text is kerned and has ligatures since
  2026-10-25 ([ADR 0029](0029-kerning-and-ligatures.md)); glyphs are
  grid-fitted (hinted), `light` by default, since 2026-11-02 (a
  `core.text-hinting` setting chooses it, and packs record their mode)
- Date: 2026-10-14

## Context

PaperPaw needs good-looking, anti-aliased text on every device. Larger
devices should render any OpenType/TrueType font at any size. The ESP32-C3
has no FPU and no atomic compare-and-swap; `skrifa`/`read-fonts` (the
maintained font reader from Google Fonts) depend on `alloc::sync::Arc`,
which does not exist on that target. The popular `ttf-parser` is no longer
maintained (RUSTSEC-2026-0192).

## Decision

- `paperpaw-text` provides one `Font` type with two backends:
  - **Outline** (feature `outline`): `skrifa` reads outlines at runtime, a
    built-in accumulation rasterizer renders 4-bit anti-aliased glyphs, and
    a size-bounded LRU glyph cache keeps page turns cheap.
  - **Bitmap packs** (`.ppfp`, always available): glyphs pre-rendered per
    size, read in place from flash or storage without RAM or floating-point
    work.
- Packs are produced by `paperpaw-fontgen` (and by the `fonts-default`
  build script) with the *same* rasterizer, so text renders pixel-identical
  on both paths; a test enforces this.
- Glyphs are grid-fitted to the screen's pixels (2026-11-02). A scalable
  font is fitted when drawn: a `HintingInstance` kept per size and mode,
  the fitting never moves an advance (`preserve_linear_metrics`), so lines
  and pages stay what they were — the same output re-inked. `light` comes
  default (a setting, `core.text-hinting`, may turn it off or make it
  `normal`); a pack keeps how it was drawn, and the header says which
  (the 2 reserved bytes then the hinting byte; packs made before the
  change have 0 there, which reads as `off` — the version stays 1).
- Fonts are contributions (`FontSpec`) bound to roles (`core.font-ui`,
  `core.font-reading`) like screens to slots; users rebind roles in
  `[fonts]`.
- Default fonts: Literata (reading) and Atkinson Hyperlegible Next (UI),
  both OFL-1.1, shipped by the `fonts-default` plugin.

## Consequences

- Any device can show any font; small devices are limited to the sizes and
  character sets in their packs (default: Latin, 20 to 48 px).
- No kerning or complex shaping yet; a shaping step (e.g. HarfRust) can be
  added for Tier L later without changing the font API.
- Variable-font features such as optical size are not yet applied per size.
