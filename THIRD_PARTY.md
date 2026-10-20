# Third-party material bundled in this repository

## Lucide icons

- What: 38 icons (SVG, 24x24, stroke) used by the browser designer (inlined into `web/index.html`) and the native
  studio (`crates/gf-studio`, drawn as vector paths from the same files).
- Where: `third_party/lucide/icons/`, copied verbatim from the npm package `lucide-static` version 1.60.0
  (https://registry.npmjs.org/lucide-static/-/lucide-static-1.60.0.tgz, npm integrity
  sha512-Y8/IYjBYST40TuqzknGt1XJOcZp83UH5fLndgXlQCQ4lM97eZO3EQo6cIyDcBrprredIGDVxge8yslRyM4wD0Q==).
- Project: https://lucide.dev, source https://github.com/lucide-icons/lucide (tag 1.60.0, commit
  02eb9161b57917ccaa262f1bf070507cddde3b08).
- Licence: ISC License, Copyright (c) 2026 Lucide Icons and Contributors; the icons Lucide lists as derived from Feather
  are also under the MIT License, Copyright (c) 2013-present Cole Bemis. The full text, identical to
  https://github.com/lucide-icons/lucide/blob/1.60.0/LICENSE and to the package's LICENSE, is kept in
  `third_party/lucide/LICENSE`.
- Bundled icons on Lucide's Feather list, so covered by both notices: `crosshair`, `moon`, `square`, `x`, plus
  `circle-help`, `columns-2` and `triangle-alert`, which are Lucide's current names for the listed `help-circle`,
  `columns` and `alert-triangle` (the package ships both names with identical drawings).
- Regenerate the sprite and the studio's table with `python tools/icons/build_icons.py` (the list of icons lives there).
