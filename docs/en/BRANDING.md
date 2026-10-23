# KERN style

Approved on 2026-10-23: **Керн / KERN**, the three-layer mark and amber **Ember / Gruvbox** palette. Vector: [kern.svg](../../assets/kern.svg); tokens: [kern-theme.json](../../assets/kern-theme.json). The first native screen uses the background/surface/text/amber accent; full mark and styling integration remains ahead.

| Role | Color |
|---|---|
| Background / surface | `#1d2021` / `#282828` |
| Text / secondary | `#ebdbb2` / `#bdae93` |
| Accent | `#fabd2f` |
| Border | `#665c54` |
| Success / error | `#b8bb26` / `#fb4934` |

A dense, readable working interface: thin separators, few borders/repeated labels, widgets can hide or collapse. Fine chart grids use subtly different neighboring background tiles rather than fine lines. Major divisions/time follow samples; the pen meets the latest curve point. Double left-click places/removes a flag; nearby curves are selected by distance, ambiguity opens a compact chooser. Transparency affects panel backgrounds only; text/controls remain crisp. Blur is deferred.

Colors are inspired by [Gruvbox](https://github.com/morhetz/gruvbox); theme sources are not bundled. Glow, constant animation and decorative load are not goals; resources must be measured on low-end target hardware.
