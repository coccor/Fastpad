# Licenses

## FastPad

FastPad source code and `FastPad.exe`: MIT License, Copyright (c) 2026 Cocioaba Cornel. The full
text is in `LICENSE`.

## Native components

| Component | Version | License | Text |
|---|---|---|---|
| Scintilla (`Scintilla.dll`) | 5.6.6 | License for Lexilla, Scintilla, and SciTE (Historical Permission Notice and Disclaimer style), Copyright 1998-2021 Neil Hodgson | `licenses/Scintilla.txt` |
| Lexilla (`Lexilla.dll`) | 5.5.3 | License for Lexilla, Scintilla, and SciTE (Historical Permission Notice and Disclaimer style), Copyright 1998-2021 Neil Hodgson | `licenses/Lexilla.txt` |

## Bundled artwork

| Artwork | Version | License | Text |
|---|---|---|---|
| Material Icon Theme icons (26 file and folder icons, embedded in `FastPad.exe`) | 5.38.1 | MIT License, Copyright (c) 2025 Material Extensions | `licenses/material-icon-theme.txt` |
## Rust crates

FastPad depends directly on windows-sys 0.61.2, serde_json 1.0.151, pulldown-cmark 0.13.4, regex
1.13.1, windows 0.62.2, windows-numerics 0.3.1, and resvg 0.48.1. The complete locked dependency
closure, with the license expressions reported by `cargo metadata --locked`, is:

| Crate | Version | License | Role |
|---|---|---|---|
| `windows-sys` | 0.61.2 | MIT OR Apache-2.0 | Win32 bindings (linked) |
| `windows-link` | 0.2.1 | MIT OR Apache-2.0 | Import linking for `windows-sys` (linked) |
| `serde_json` | 1.0.151 | MIT OR Apache-2.0 | Explicit JSON commands (linked) |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | Locked but not linked (no normal dependency edge for this target) |
| `serde_core` | 1.0.229 | MIT OR Apache-2.0 | `serde_json` dependency (linked) |
| `itoa` | 1.0.18 | MIT OR Apache-2.0 | `serde_json` dependency (linked) |
| `memchr` | 2.8.3 | Unlicense OR MIT | `serde_json`, `regex`, and `resvg` dependency (linked) |
| `zmij` | 1.0.23 | MIT | `serde_json` dependency (linked) |
| `serde_derive` | 1.0.229 | MIT OR Apache-2.0 | Build-time procedural macro |
| `proc-macro2` | 1.0.107 | MIT OR Apache-2.0 | Build-time procedural macro support |
| `quote` | 1.0.47 | MIT OR Apache-2.0 | Build-time procedural macro support |
| `syn` | 3.0.4 | MIT OR Apache-2.0 | Build-time procedural macro support |
| `unicode-ident` | 1.0.24 | (MIT OR Apache-2.0) AND Unicode-3.0 | Build-time procedural macro support |
| `pulldown-cmark` | 0.13.4 | MIT | Markdown preview parser (linked) |
| `bitflags` | 2.13.2 | MIT OR Apache-2.0 | `pulldown-cmark` and `resvg` dependency (linked) |
| `unicase` | 2.9.0 | MIT OR Apache-2.0 | `pulldown-cmark` dependency (linked) |
| `regex` | 1.13.1 | MIT OR Apache-2.0 | Note text search (linked) |
| `regex-automata` | 0.4.18 | MIT OR Apache-2.0 | `regex` dependency (linked) |
| `regex-syntax` | 0.8.11 | MIT OR Apache-2.0 | `regex` dependency (linked) |
| `aho-corasick` | 1.1.5 | Unlicense OR MIT | `regex` dependency (linked) |
| `windows` | 0.62.2 | MIT OR Apache-2.0 | Direct2D/DirectWrite/WIC interface bindings (linked) |
| `windows-core` | 0.62.2 | MIT OR Apache-2.0 | Direct2D/DirectWrite/WIC interface bindings (linked) |
| `windows-result` | 0.4.1 | MIT OR Apache-2.0 | Direct2D/DirectWrite/WIC interface bindings (linked) |
| `windows-strings` | 0.5.1 | MIT OR Apache-2.0 | Direct2D/DirectWrite/WIC interface bindings (linked) |
| `windows-numerics` | 0.3.1 | MIT OR Apache-2.0 | Direct2D/DirectWrite/WIC interface bindings (linked) |
| `windows-future` | 0.3.2 | MIT OR Apache-2.0 | `windows` dependency (linked only if referenced) |
| `windows-threading` | 0.2.1 | MIT OR Apache-2.0 | `windows` dependency (linked only if referenced) |
| `windows-collections` | 0.3.2 | MIT OR Apache-2.0 | `windows` dependency (linked only if referenced) |
| `windows-implement` | 0.60.2 | MIT OR Apache-2.0 | Build-time procedural macro |
| `windows-interface` | 0.59.3 | MIT OR Apache-2.0 | Build-time procedural macro |
| `syn` | 2.0.119 | MIT OR Apache-2.0 | Build-time procedural macro |
| `resvg` | 0.48.1 | Apache-2.0 OR MIT | SVG image renderer (linked) |
| `usvg` | 0.48.1 | Apache-2.0 OR MIT | `resvg` SVG parser (linked) |
| `adler2` | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `arrayref` | 0.3.9 | BSD-2-Clause | `resvg` dependency (linked) |
| `arrayvec` | 0.7.8 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `autocfg` | 1.5.1 | Apache-2.0 OR MIT | Build script support |
| `base64` | 0.23.1 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | `resvg` dependency (linked) |
| `bytemuck_derive` | 1.12.1 | Zlib OR Apache-2.0 OR MIT | Build-time procedural macro |
| `byteorder-lite` | 0.1.0 | Unlicense OR MIT | `resvg` dependency (linked) |
| `cfg-if` | 1.0.5 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `color_quant` | 1.1.0 | MIT | `resvg` dependency (linked) |
| `crc32fast` | 1.5.2 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `data-url` | 0.3.2 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `euclid` | 0.22.14 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `fdeflate` | 0.3.7 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `flate2` | 1.1.10 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `float-cmp` | 0.9.0 | MIT | `resvg` dependency (linked) |
| `font-types` | 0.12.6 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `fontdb` | 0.24.0 | MIT | `resvg` dependency (linked) |
| `gif` | 0.14.2 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `harfrust` | 0.12.0 | MIT | `resvg` dependency (linked) |
| `image-webp` | 0.2.4 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `imagesize` | 0.15.0 | MIT | `resvg` dependency (linked) |
| `kurbo` | 0.13.1 | Apache-2.0 OR MIT | `resvg` dependency (linked) |
| `log` | 0.4.34 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `memmap2` | 0.9.11 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `miniz_oxide` | 0.8.9 | MIT OR Zlib OR Apache-2.0 | `resvg` dependency (linked) |
| `miniz_oxide` | 0.9.1 | MIT OR Zlib OR Apache-2.0 | `resvg` dependency (linked) |
| `num-traits` | 0.2.19 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `once_cell` | 1.21.4 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `pico-args` | 0.5.0 | MIT | `resvg` dependency (linked) |
| `png` | 0.18.1 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `polycool` | 0.4.0 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `quick-error` | 2.0.1 | MIT/Apache-2.0 | `resvg` dependency (linked) |
| `read-fonts` | 0.41.0 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `rgb` | 0.8.53 | MIT | `resvg` dependency (linked) |
| `roxmltree` | 0.21.1 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `simd-adler32` | 0.3.10 | MIT | `resvg` dependency (linked) |
| `simplecss` | 0.2.2 | Apache-2.0 OR MIT | `resvg` dependency (linked) |
| `siphasher` | 1.0.4 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `skrifa` | 0.44.0 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `slotmap` | 1.1.1 | Zlib | `resvg` dependency (linked) |
| `smallvec` | 1.16.3 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `strict-num` | 0.1.1 | MIT | `resvg` dependency (linked) |
| `svgtypes` | 0.16.1 | Apache-2.0 OR MIT | `resvg` dependency (linked) |
| `tiny-skia` | 0.12.0 | BSD-3-Clause | `resvg` dependency (linked) |
| `tiny-skia-path` | 0.12.0 | BSD-3-Clause | `resvg` dependency (linked) |
| `tinyvec` | 1.13.3 | Zlib OR Apache-2.0 OR MIT | `resvg` dependency (linked) |
| `unicode-bidi` | 0.3.18 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `unicode-script` | 0.5.8 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `unicode-vo` | 0.1.0 | MIT/Apache-2.0 | `resvg` dependency (linked) |
| `version_check` | 0.9.5 | MIT/Apache-2.0 | Build script support |
| `weezl` | 0.1.12 | MIT OR Apache-2.0 | `resvg` dependency (linked) |
| `xmlwriter` | 0.1.0 | MIT | `resvg` dependency (linked) |
| `zlib-rs` | 0.6.8 | Zlib | `resvg` dependency (linked) |
| `zune-core` | 0.5.3 | MIT OR Apache-2.0 OR Zlib | `resvg` dependency (linked) |
| `zune-jpeg` | 0.5.15 | MIT OR Apache-2.0 OR Zlib | `resvg` dependency (linked) |

FastPad uses these crates under the MIT license option where a choice is offered; `unicode-ident`
additionally carries the Unicode-3.0 license for its Unicode data tables. Without an MIT option,
`arrayref` is used under BSD-2-Clause, `tiny-skia` and `tiny-skia-path` under BSD-3-Clause, and
`slotmap` and `zlib-rs` under the Zlib license. Build-time procedural macro and build script crates
are not linked into `FastPad.exe`.

The portable package includes `licenses/rust-crates.txt`, generated during packaging by
`tools/rust-crate-licenses.ps1` from `cargo metadata --locked`. It lists every crate linked into
`FastPad.exe` with its version and SPDX license, followed by the verbatim `LICENSE*`/`COPYING`
files, including copyright notices, from that crate's source.
