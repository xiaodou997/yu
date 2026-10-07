<p align="center">
  <img src="./platform/macos/yu-shell-macos/Assets/branding/yu-logo-master-1024.png" width="160" alt="Yu Markdown Logo" />
</p>

<h1 align="center">Yu Markdown</h1>

<p align="center">
  <strong>A native, Markdown-first visual editor powered by Rust.</strong>
</p>

<p align="center">
  <a href="./README.md">English</a>
  ·
  <a href="./README.zh-CN.md">简体中文</a>
  ·
  <a href="./README.zh-TW.md">繁體中文</a>
  ·
  <a href="./README.ja-JP.md">日本語</a>
  ·
  <a href="./README.ko-KR.md">한국어</a>
</p>

<p align="center">
  <a href="https://github.com/xiaodou997/yu/releases/latest">
    <img src="https://img.shields.io/github/v/release/xiaodou997/yu?style=flat-square&color=blue" alt="Latest release" />
  </a>
  <a href="https://github.com/xiaodou997/yu/actions/workflows/ci.yml">
    <img src="https://github.com/xiaodou997/yu/actions/workflows/ci.yml/badge.svg" alt="CI" />
  </a>
  <a href="https://github.com/xiaodou997/yu/blob/main/LICENSE">
    <img src="https://img.shields.io/badge/License-Apache--2.0-green?style=flat-square" alt="Apache-2.0" />
  </a>
</p>

Yu Markdown keeps **Markdown source as the single source of truth** while giving you a rendered, direct-editing experience. The shared editor core is written in Rust; macOS and Windows use native platform shells, native input systems, and GPU rendering instead of a WebView or Chromium runtime.

## Highlights

- **Markdown-first editing** — source text is the canonical document model; there is no rich-text round-trip serialization.
- **Visual direct editing** — rendered content stays editable through real-time source projection and decorations.
- **Native desktop experience** — Swift/AppKit + Metal on macOS; Win32 + DirectWrite/D3D11 + TSF/IME on Windows.
- **Fast incremental engine** — parsing, layout, scene updates, and rendering are revision-bound and only recompute changed or visible content.
- **International text support** — CJK, emoji, combining characters, bidi/RTL text, and native IME composition are first-class concerns.
- **Local-first files** — your Markdown documents remain ordinary files on your machine, with external-change and save-conflict handling.
- **Rich Markdown workflows** — code blocks, tables, math, diagrams, images, clipboard/export flows, and system printing are handled without changing the Markdown source-of-truth model.
- **Open source** — licensed under Apache-2.0.

## Download

Get the current macOS and Windows builds from **[GitHub Releases](https://github.com/xiaodou997/yu/releases/latest)**.

| Platform | Current support |
| --- | --- |
| macOS | Apple Silicon, macOS 26 or newer. Release builds are Developer ID signed and Apple-notarized. |
| Windows | x64, Windows 10 2004 or newer. Installer and portable builds are available; current releases are not Authenticode-signed. |
| Linux | The shared Rust core is validated, but a desktop package is not published yet. |

## How it works

Yu is intentionally not a WebView Markdown editor, HTML editor, or rich-text editor with Markdown import/export. Its architecture keeps one document truth and one rendering path:

```text
Markdown source
    ↓
Transactions + revisioned snapshots
    ↓
Incremental Markdown syntax
    ↓
Source projection + decorations
    ↓
Native layout
    ↓
Retained scene
    ↓
GPU renderer
```

Key invariants:

1. Markdown source is always the canonical document.
2. Permanent edits go through transactions.
3. Markdown semantics live in `yu-markdown`; visual state is represented by decorations.
4. Derived data is tied to a revision, and stale results are rejected.
5. IME composition is transient and never corrupts document state.
6. Platform shells do not parse Markdown.
7. Caches, async resources, and GPU state cannot change editing semantics.
8. There is no second rendering path.

## Platform architecture

- **Shared core:** Rust crates implement text storage, syntax, editor state, decorations, layout, scene construction, rendering plans, assets, storage, workspaces, and export.
- **macOS:** native Swift/AppKit shell, CoreText integration, Metal rendering, FSEvents, native menus, input, accessibility, and window lifecycle.
- **Windows:** native Win32 shell built with Rust/windows-rs, DirectWrite text services, D3D11 rendering, TSF/IME input, DPI/theme handling, and native file lifecycle.
- **Linux:** currently focused on validating the shared core.

## Repository layout

```text
crates/yu-core          shared coordinates, revisions, anchors
crates/yu-text          Rope, snapshots, transactions
crates/yu-syntax        incremental CST
crates/yu-markdown      Markdown semantics and extensions
crates/yu-state         editor state, history, facets
crates/yu-decoration    source ↔ visual projection
crates/yu-layout        line/widget layout, bidi, hit testing
crates/yu-scene         retained scene and damage tracking
crates/yu-render        backend-neutral render plans
crates/yu-assets        image/resource scheduling and caches
crates/yu-storage       Markdown sessions, atomic saves, file watching
crates/yu-workspace     tabs and document-session lifecycle
crates/yu-export        revision-bound clipboard and HTML export

platform/macos          native macOS shell, font, render, storage adapters
platform/windows        native Windows shell, packaging, installer
tools                   verification, inspection, benchmarks, release tooling
```

## Development

Yu is pinned to Rust 1.98.1.

```bash
git clone https://github.com/xiaodou997/yu.git
cd yu

# Full verification
tools/verify.sh

# Rust-only checks
tools/verify.sh --rust-only

# macOS: build and launch the native app
platform/macos/yu-shell-macos/run-app.sh README.md
```

On Windows:

```powershell
./platform/windows/yu-shell-windows/run-self-checks.ps1
```

For FFI changes on macOS, use a clean build to avoid stale SwiftPM artifacts:

```bash
platform/macos/yu-shell-macos/run-self-checks.sh --clean-build
```

## Support and privacy

- Bug reports and feature requests: [GitHub Issues](https://github.com/xiaodou997/yu/issues)
- Releases: [GitHub Releases](https://github.com/xiaodou997/yu/releases/latest)
- macOS privacy: [PRIVACY.md](PRIVACY.md)
- Windows privacy: [PRIVACY-WINDOWS.md](PRIVACY-WINDOWS.md)

## Contributing

Issues and pull requests are welcome. Changes to editor behavior should include behavioral tests; incremental algorithms should be checked against their full equivalents. Run the local verification suite before submitting platform or core changes.

## License

Yu Markdown is open source under the [Apache License 2.0](LICENSE).
