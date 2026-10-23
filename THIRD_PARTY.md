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

## MakeHuman rigs and skin weights

- What: MakeHuman's eight standard rigs and seven weight sets (JSON), used by `export-glb --body-rig` and
  `body.joints`; seven rig/weight pairs are built into the binary as presets (`crates/gf-core/src/rig_presets.rs`);
  `rig.default_no_toes.json` has no weights of its own and is shipped but not a preset.
- Where: `crates/gf-core/makehuman/rigs/*.json.gz` (inside the crate that embeds them), each a gzip (no name, mtime 0, level 9) of the original file, byte for
  byte, made by `tools/third_party/pack_makehuman_rigs.py`.
- Source: MPFB 2 (https://github.com/makehumancommunity/mpfb2), commit d0a32e57a7f915cb2f2b95410e2117648c7bbb7e
  (2026-10-04), folder `src/mpfb/data/rigs/standard/`.
- Licence: CC0 1.0 Universal. MPFB's LICENSE.md (section C) defines its assets as "any data contributing to the output
  from MPFB", listing "Rigs, poses and expressions" and "JSON data with mesh information", released under CC0 1.0; each
  weights file also states `"license": "CC0"` itself. The legal text is kept in `crates/gf-core/makehuman/LICENSE.ASSETS.md`
  (MPFB's LICENSE.ASSETS.md, verbatim). MPFB's code is GPLv3 and none of it is used here.

| File | Bytes | sha256 (original) | Gzipped | sha256 (gzipped, as shipped) |
|---|---|---|---|---|
| `rig.cmu_mb.json` | 23,803 | `a751d730091304de2a7f7407a57976e86f71f75125e21fa2626ca5c661a6ad68` | 2,201 | `16318bb8f03d0cf30f4fe862d740612847ade6121028f533d339787a367c39ae` |
| `rig.default.json` | 126,355 | `8b949de35c2dd70dbb5094e57a8586c3b9a9f650775a293212581a8f67be6797` | 9,480 | `a864f8717fc053a8ce688c28c7b601ae9981c268bfd4c6a5fb746a0edddddd96` |
| `rig.default_no_toes.json` | 106,048 | `6b178e6924b9af2a76db29eb507b2f77ecff53f93c5477b043d0d7f33e93ef69` | 8,064 | `36396cc7540394a55ffa453212cf8ded7d8a826356f9cb43fc6c2484c39364e4` |
| `rig.game_engine.json` | 40,936 | `b324ddb4b707721c19630bfadb87c3e3b38eed3b6e4590bb3cead43ea918f799` | 3,381 | `1963f04614971a838210b7e453b76e228cf3ae00766239eb68a9fb6067034a34` |
| `rig.game_engine_with_breast.json` | 42,438 | `02f51719f8420af6568c6ab6123f0998040ca9f589a9173af3087cbd2a11839f` | 3,523 | `c70bba16cdbe044be7744324c8484ae239260f4fad3bd1f06985fb52399d9112` |
| `rig.mixamo.json` | 84,107 | `b4e491bacdcae797e53b7692169cf6a160c50fcc1eda5f0982e92fac1561ae56` | 2,939 | `a51866d8cb4b050752e9103df28a8fdd39b99da5c8a441623867d1b6d0037efa` |
| `rig.mixamo_unity.json` | 50,905 | `c991d9e61090c6d94a8fd4605ca5285b1b087aa73177a6374169b9b034643ea8` | 4,101 | `773cd2b99d285dbf0bc50c6ec28f0f535d54b3c72592ee8009285984e154fca5` |
| `rig.openpose.json` | 21,961 | `b841bbeb452532f59e119531c55a750ce05b7e1c3db247af7c137c7a9d4b7cc4` | 1,299 | `faff4fbd82c46204dd6be124ed74b906a4dfe483045d93cff8775f3acc430ca2` |
| `weights.cmu_mb.json` | 3,024,572 | `e349f27b120b848858fca21c386b257de0b93e7ec26113f6dd2e8caa8ce11e9b` | 350,979 | `3d31a8a8ea28e33b1862c5790af6092cd4e44c7e6dafd0ed10c13f33254cf6de` |
| `weights.default.json` | 5,034,981 | `154b866774a8c2b055a8e86419f22a87b76c60440fdcb70bcb78345f00924e89` | 631,265 | `26348b3aa7655d22d8d4f5294f4612e29fdb917612c8b0069d7f41adced190b8` |
| `weights.game_engine.json` | 3,026,778 | `9f4a773e74ce5ba08415b4f30070338e9f75bae5971b5075b10cd6b9caa58473` | 348,410 | `b08eaf4e887724c3f9cad6eee8315c5df684b11ff1e1cad8c550465963a8c59f` |
| `weights.game_engine_with_breast.json` | 3,103,207 | `0c82f80d5bcb46bd71ea76f6a2ef0db5ca31f048df401e280b4497f5c66c64f7` | 360,315 | `3703b0c74ba8e30b4a16069bc852c3d5606780c72355f0fee861f2720b26583a` |
| `weights.mixamo.json` | 2,372,203 | `4561be4d7c0093d70e3dba4fbacbbbb6d66781573779fb12a0509cefa181b8ae` | 153,868 | `3a112fd47a195275331b960192b5ccbde5cdf283bbc570cc9d3285491836d78e` |
| `weights.mixamo_unity.json` | 3,342,705 | `0cfa562682fe27dab69cd3abd287cc971fbe576e03297d702798ac85813fb102` | 390,493 | `2e9063ce0ab36228ccebc7b73fe04fc63988d15a04582b5b18a10b0dc3ed3d09` |
| `weights.openpose.json` | 2,634,893 | `3f3f0b6fb208e0316fd3663b3559ee649b765969ad220347585ace3090e947ac` | 196,507 | `26a902c083c2169c64bf4890a50e216f71636e3484e57970d8d7c664a61c05a1` |

## Rust crates compiled into `web/gf_wasm.wasm`

The browser designer's WebAssembly module (`crates/gf-wasm`) is a compiled binary in this repository, so it carries
the code of the crates it links. Its runtime dependencies (`cargo tree -p gf-wasm -e normal --target
wasm32-unknown-unknown`; `serde_derive`, `syn`, `quote`, `proc-macro2` and `unicode-ident` only run at build time):

| Crate | Version | Authors | Licence | Used under |
|---|---|---|---|---|
| `serde` | 1.0.229 | Erick Tryzelaar, David Tolnay | MIT OR Apache-2.0 | MIT |
| `serde_core` | 1.0.229 | Erick Tryzelaar, David Tolnay | MIT OR Apache-2.0 | MIT |
| `serde_json` | 1.0.154 | Erick Tryzelaar, David Tolnay | MIT OR Apache-2.0 | MIT |
| `itoa` | 1.0.18 | David Tolnay | MIT OR Apache-2.0 | MIT |
| `zmij` | 1.0.23 | David Tolnay | MIT | MIT |
| `memchr` | 2.8.3 | Andrew Gallant, bluss | Unlicense OR MIT | Unlicense (no notice required) |

The five crates used under MIT ship the same MIT text (sha256
`23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3`), kept in `third_party/rust-wasm/LICENSE-MIT`.

The wasm also carries Rust's own runtime from the toolchain that built it (rustc 1.93.1, target
`wasm32-unknown-unknown`; the wasm's strings name `library/core`, `library/std` and `dlmalloc-0.2.11`):

| Component | Version | Authors | Licence | Used under | Text |
|---|---|---|---|---|---|
| Rust standard library (`core`, `alloc`, `std`) | rustc 1.93.1 | The Rust Project Developers | MIT OR Apache-2.0 | MIT | `third_party/rust-wasm/LICENSE-MIT` (the same text) |
| `dlmalloc` (the wasm allocator) | 0.2.11 | Alex Crichton | MIT OR Apache-2.0 | MIT | `third_party/rust-wasm/dlmalloc-LICENSE-MIT` (copied from the crate, sha256 of the `.crate` `06cdfe34...fa30` as locked by the standard library) |
| `compiler_builtins` (linked by the standard library) | 0.1.160 | The Rust Project Developers and LLVM contributors | MIT AND Apache-2.0 WITH LLVM-exception AND (MIT OR Apache-2.0) | as stated | `third_party/rust-wasm/compiler-builtins-LICENSE.txt` (from the toolchain's `library/compiler-builtins/LICENSE.txt`) |

The native binaries (`gazeforge`, `gazeforge-studio`) are not committed here. A release of them needs its own notice
for their much larger dependency tree, including the Unicode-3.0 data tables in `unicode-normalization`.
