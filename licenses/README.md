# Third-party native component licenses

`Scintilla.txt` is copied from Scintilla 5.6.6 and `Lexilla.txt` is copied
from Lexilla 5.5.3 by `tools/build-native.ps1`. Both files are kept in source
control so the portable package can include the upstream license texts.

`material-icon-theme.txt` is the MIT License of Material Icon Theme 5.38.1, whose icons are
embedded in FastPad.exe (see `assets/icons/material/SOURCE.md`).

`FastDraw.txt` is the MIT License of the FastDraw renderer (`fastdraw-render`, pinned by git commit
`f0d3905`), which draws `.fdraw` diagrams. It ships Phosphor Icons, whose MIT License is in
`phosphor-icons.txt`, and fonts under the SIL Open Font License 1.1: `OFL-Geist.txt` (Geist and Geist
Mono), `OFL-InstrumentSerif.txt`, `OFL-LibreFranklin.txt`, `OFL-Newsreader.txt` and `OFL-SketchHand.txt`
(Sketch Hand, a modified Virgil). These texts are copied from that commit's `LICENSE`, `licenses/` and
`fonts/`.
