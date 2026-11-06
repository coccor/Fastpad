# Installing FastPad

```powershell
winget install coccor.FastPad
```

| Method | How | What you get |
|---|---|---|
| **winget** | `winget install coccor.FastPad` | Per-user install; `winget upgrade` keeps it current |
| **Scoop** | `scoop bucket add fastpad https://github.com/coccor/FastPad`<br>`scoop install fastpad/fastpad` | Portable install, Start menu shortcut, `fastpad` command |
| **Installer** | [`FastPad-<version>-windows-x64-setup.exe`](https://github.com/coccor/FastPad/releases/latest) | Per-user install, no administrator prompt |
| **Portable** | [`FastPad-<version>-windows-x64.zip`](https://github.com/coccor/FastPad/releases/latest) | Extract anywhere, run `FastPad.exe`; nothing written to the registry |

Every release publishes `SHA256SUMS.txt` so you can verify what you downloaded.

## Installer

The installer puts FastPad in `%LocalAppData%\Programs\FastPad` and only touches your own user
registry: an uninstall entry, a Start menu shortcut, `fastpad` for Win+R, and FastPad in the
"Open with" list for `.txt`, `.md`, `.markdown`, `.json`, `.log` and `.ini`. Your default apps are
never changed. Optional extras: a desktop shortcut and "Edit with FastPad" on every file's right-click
menu. Uninstall from Settings > Apps. Unattended install:

```powershell
FastPad-<version>-windows-x64-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /TASKS="contextmenu"
```

`/ALLUSERS` installs for every user into Program Files instead (requires elevation).

## Portable ZIP

The portable ZIP holds `FastPad.exe`, `Scintilla.dll` and `Lexilla.dll` (keep them together),
plus `README.md`, `LICENSE`, `LICENSES.md` and `licenses\`.

Settings and recovery snapshots live in `%LocalAppData%\FastPad` whichever way you install, so you
can switch methods without losing them, and uninstalling keeps them.

## Portable mode

Portable mode keeps them beside `FastPad.exe` instead, for a copy on a USB stick or in a tools
folder: put an empty file named `portable` (no extension) next to `FastPad.exe`, and FastPad keeps
its settings, session and recovery snapshots in a `data` folder there. The folder must be
writable, so don't do this in an installed copy under Program Files. A portable FastPad runs
separately from an installed one: files opened from either go to its own window.

## "Windows protected your PC"

Current releases aren't code-signed yet, so SmartScreen may warn on first run. Choose
**More info > Run anyway**, or check the file against `SHA256SUMS.txt` first.
