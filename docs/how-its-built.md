# How FastPad is built, and why it's fast

There is no single trick. FastPad is fast because of everything it doesn't do before you can type.

- **Nothing between you and Windows.** FastPad is one Rust executable that calls the Win32 API
  directly. There's no runtime to start, no JIT, no garbage collector and no browser engine. The C
  runtime is linked in statically, so the executable has no runtime DLLs of its own to resolve.
- **A proven engine for the text.** The editing surface is
  [Scintilla](https://www.scintilla.org/), the C++ engine behind Notepad++ and SciTE, loaded from
  `Scintilla.dll` next to the executable. `Lexilla.dll`, which does the syntax highlighting, isn't
  loaded at all until a file needs a lexer.
- **The window first.** At launch FastPad does the minimum it needs to show an editor: read
  `fastpad.ini`, create the window and the Scintilla control, give it focus. As soon as that frame
  paints, you can type. If FastPad is already running, a new launch hands its files to the open
  window over a named pipe and exits before loading anything.
- **The rest of startup gets out of your way.** Everything else runs after the first paint as a
  chain of small steps on the window's message queue: apply your settings, restore your session,
  open your notebook, open the file you launched with, turn on syntax highlighting, look for crash
  recovery snapshots, start listening for other launches, build the remaining chrome. Before each
  step, FastPad checks for keyboard or mouse input. If you're typing, the step goes to the back of
  the queue and runs later. Session restore reopens one tab per step, so a long session never
  freezes the window. The message loop also handles pending input before anything else.
- **Heavy work stays off the UI thread, or doesn't happen.** Scanning the notebook, full-text
  search, full Markdown preview parses, and decoding images and SVGs all run on worker threads
  that post their results back. Direct2D, DirectWrite and the GPU driver load the first time you open the Markdown preview,
  and not before, which is a big part of why idle memory stays around 2 MB. Recovery snapshots are
  written on a timer, only once you've paused typing for two seconds.
- **A small, pinned dependency tree.** Eight direct crates (`windows-sys`, `windows`, `serde_json`,
  `pulldown-cmark`, `regex`, `resvg`, `mermaid-rs-renderer`, `windows-numerics`), each pinned to an
  exact version.
  [`tools/audit-dependencies.ps1`](../tools/audit-dependencies.ps1) fails if anything else gets in.
  Release builds use fat LTO, one codegen unit and `panic = "abort"`. Code that never runs at
  startup, like the SVG renderer, is optimized for size instead of speed.
- **Measured, not assumed.** [`tools/benchmark.ps1`](../benchmarks/README.md) launches the release
  build 100 times. For each launch it types a character and waits until Scintilla has drawn it on
  screen, then records nine startup milestones and the idle memory. `fastpad-bench compare` counts a
  change as a regression only when p95 gets worse by at least 2 ms or 10% and a bootstrap
  confidence interval rules out noise.
