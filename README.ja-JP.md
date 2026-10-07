<p align="center">
  <img src="./platform/macos/yu-shell-macos/Assets/branding/yu-logo-master-1024.png" width="160" alt="Yu Markdown Logo" />
</p>

<h1 align="center">Yu Markdown</h1>

<p align="center">
  <strong>Rust で構築された、ネイティブかつ Markdown-first のビジュアルエディター。</strong>
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

Yu Markdown は **Markdown ソースを唯一の正本**として維持しながら、レンダリング結果をそのまま編集できる体験を提供します。共有エディターコアは Rust で実装され、macOS と Windows は WebView / Chromium / 常駐 JavaScript runtime に依存せず、ネイティブ UI、ネイティブ入力、GPU レンダリングを使用します。

## 主な特徴

- **Markdown-first** — Markdown テキスト自体が文書モデルであり、リッチテキストとの往復変換を行いません。
- **レンダリング状態で直接編集** — リアルタイム Source Projection と Decoration により、見たままの内容を直接編集できます。
- **ネイティブデスクトップ** — macOS は Swift/AppKit + Metal、Windows は Win32 + DirectWrite/D3D11 + TSF/IME。
- **インクリメンタルな高性能コア** — 構文、レイアウト、シーン、描画は Revision に紐づき、変更部分と可視部分だけを再計算します。
- **国際化入力を重視** — CJK、emoji、結合文字、bidi/RTL、ネイティブ IME composition を第一級の要件として扱います。
- **ローカルファースト** — Markdown 文書は通常のローカルファイルのまま保持され、外部変更や保存競合も扱います。
- **Markdown ワークフロー** — コードブロック、表、数式、図、画像、クリップボード/エクスポート、システム印刷を Markdown の正本モデル上で扱います。
- **オープンソース** — Apache-2.0。

## ダウンロード

現在の macOS / Windows ビルドは **[GitHub Releases](https://github.com/xiaodou997/yu/releases/latest)** から取得できます。

| プラットフォーム | 現在のサポート |
| --- | --- |
| macOS | Apple Silicon、macOS 26 以降。正式ビルドは Developer ID 署名済みで Apple notarization 済みです。 |
| Windows | x64、Windows 10 2004 以降。インストーラー版とポータブル版を提供。現行リリースは Authenticode 未署名です。 |
| Linux | 共有 Rust コアは検証済みですが、デスクトップパッケージはまだ公開していません。 |

## 仕組み

Yu は WebView ベースの Markdown エディターでも、HTML エディターでも、「リッチテキスト + Markdown 入出力」でもありません。文書の正本と描画経路をそれぞれ一つに保ちます。

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

主要な不変条件：

1. Markdown source は常に唯一の正本です。
2. 永続的な編集はすべて Transaction を通ります。
3. Markdown の意味論は `yu-markdown` に集約し、表示状態は Decoration で表現します。
4. 派生データは Revision に紐づき、古い結果は破棄します。
5. IME composition は transient overlay として扱い、文書状態を汚しません。
6. プラットフォーム層は Markdown を解析しません。
7. キャッシュ、非同期リソース、GPU 状態は編集意味論を変えません。
8. 第二の描画経路はありません。

## プラットフォーム構成

- **共有コア**：Rust crates がテキスト、構文、エディター状態、Decoration、レイアウト、Scene、RenderPlan、アセット、ストレージ、ワークスペース、エクスポートを担当します。
- **macOS**：Swift/AppKit のネイティブシェル、CoreText、Metal、FSEvents、ネイティブメニュー、入力、Accessibility、ウィンドウ管理。
- **Windows**：Rust/windows-rs + Win32 のネイティブシェル、DirectWrite、D3D11、TSF/IME、DPI/テーマ、ファイルライフサイクル。
- **Linux**：現在は共有コアの検証が中心です。

## リポジトリ構成

```text
crates/yu-core          coordinates, Revision, Anchor
crates/yu-text          Rope, Snapshot, Transaction
crates/yu-syntax        incremental CST
crates/yu-markdown      Markdown semantics and extensions
crates/yu-state         EditorState, History, Facet
crates/yu-decoration    source ↔ visual projection
crates/yu-layout        layout, bidi, hit testing
crates/yu-scene         retained scene, damage tracking
crates/yu-render        backend-neutral RenderPlan
crates/yu-assets        image/resource scheduling and caches
crates/yu-storage       Markdown sessions, atomic save, file watching
crates/yu-workspace     tabs and document-session lifecycle
crates/yu-export        revision-bound clipboard and HTML export

platform/macos          native macOS adapters and shell
platform/windows        native Windows shell, packaging, installer
tools                   verification, inspection, benchmarks, release tools
```

## 開発

Rust 1.98.1 に固定されています。

```bash
git clone https://github.com/xiaodou997/yu.git
cd yu

tools/verify.sh
tools/verify.sh --rust-only

# macOS
platform/macos/yu-shell-macos/run-app.sh README.md
```

Windows：

```powershell
./platform/windows/yu-shell-windows/run-self-checks.ps1
```

macOS の FFI 境界を変更した場合は、SwiftPM の古い増分生成物を避けるため clean build を使用してください。

```bash
platform/macos/yu-shell-macos/run-self-checks.sh --clean-build
```

## サポートとプライバシー

- バグ報告・機能要望：[GitHub Issues](https://github.com/xiaodou997/yu/issues)
- 最新リリース：[GitHub Releases](https://github.com/xiaodou997/yu/releases/latest)
- macOS プライバシー：[PRIVACY.md](PRIVACY.md)
- Windows プライバシー：[PRIVACY-WINDOWS.md](PRIVACY-WINDOWS.md)

## コントリビュート

Issue と Pull Request を歓迎します。エディターの挙動を変更する場合は動作テストを追加し、インクリメンタルアルゴリズムは完全版との等価性を確認してください。プラットフォームまたは共有コアの変更を送る前にローカル検証を実行してください。

## License

Yu Markdown は [Apache License 2.0](LICENSE) のもとで公開されています。
