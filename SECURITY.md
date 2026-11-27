# Security

FastPad is a native Windows editor designed to stay offline. This document describes the threat model, what we harden, and how to report issues.

## Threat model

| In scope | Out of scope |
|----------|----------------|
| Malicious or untrusted note / diagram content opened locally | Remote network attackers (there is no in-process network client) |
| Same-user local abuse (second process, crafted files next to the exe) | Cross-user attacks when Windows ACLs already isolate users |
| Dependency vulnerabilities that affect parsing or rendering | Physical access or a compromised OS user account |

FastPad does **not** embed a browser or JavaScript engine. Markdown preview is laid out with Direct2D/DirectWrite; SVG/Mermaid/FastDraw are rasterized locally (resvg / in-process renderers).

## Guarantees and controls

- **No automatic network I/O.** Acceptance tests reject imports of Winsock, WinHTTP, WinInet, URLMon, and browser engines.
- **External links** (`http:`, `https:`, `mailto:`) open only after an explicit user action, via `ShellExecuteW` and the OS default handler. Link length is capped before that hand-off.
- **Remote images are never fetched.** Only local image paths are resolved for preview.
- **HTML in Markdown** is allowlisted and sanitized; `javascript:`, `data:`, and other dangerous schemes are dropped from `href` / `src`.
- **UNC and device paths** (`\\server\share`, `\\?\`, `\\.`) are rejected for local link and image resolution.
- **Absolute image paths** in note text are not auto-loaded (limits silent local file disclosure). Relative paths such as Obsidian-style `../attachments/` remain allowed. Absolute paths may still open on an explicit link click when the file exists.
- **FastDraw look files** must be relative to the diagram; absolute, UNC, drive-prefixed, and `..` segments are refused.
- **Single-instance IPC** uses a named pipe with `PIPE_REJECT_REMOTE_CLIENTS` and a DACL limited to the current user; clients verify the server process user and session before sending paths.
- **Companion DLLs** (`Scintilla.dll`, `Lexilla.dll`) load from the executable directory with `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32`. Treat a writable install/portable folder as trusted; do not plant untrusted DLLs beside the exe.

## Known residual risks

- Clicking a crafted `https:` / `mailto:` / custom protocol link runs the OS handler with attacker-influenced input (user-assisted).
- Same-user code that can connect to the FastPad named pipe can ask the primary window to open paths or folders (intended second-launch UX).
- A large Win32 `unsafe` surface is inherent to the UI stack; bugs there are memory-safety class issues, not a network RCE path by themselves.
- Parser resource exhaustion (very large or pathological Markdown/SVG/diagrams) is mitigated with size caps where practical, not eliminated.

## Dependency advisories

Run periodically:

```powershell
cargo audit
```

Markdown parsing uses `pulldown-cmark` with `ENABLE_WIKILINKS`. Keep that crate on a release that includes fixes for wikilink-related denial-of-service advisories (0.13.6 or newer).

## Reporting a vulnerability

Please open a **private** security advisory on the GitHub repository, or email the maintainers listed on the repo. Include:

1. Affected version / commit
2. Steps to reproduce
3. Impact (confidentiality, integrity, availability)

We aim to acknowledge reports promptly and to ship fixes before any public disclosure when that is practical.

## Preferred install hygiene

- Prefer the packaged installer or Winget/Scoop over an unsigned zip when possible.
- For portable builds, keep the folder non-writable by other users and verify checksums from `SHA256SUMS.txt` when provided.
- Do not run FastPad from an untrusted directory that third parties can write.
