<p align="center">
  <img src="./platform/macos/yu-shell-macos/Assets/branding/yu-logo-master-1024.png" width="160" alt="Yu Markdown Logo" />
</p>

<h1 align="center">Yu Markdown</h1>

<p align="center">
  <strong>Rust로 만든 네이티브 Markdown-first 비주얼 에디터.</strong>
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

Yu Markdown은 **Markdown 소스를 유일한 원본 데이터**로 유지하면서 렌더링된 문서를 직접 편집하는 경험을 제공합니다. 공유 편집기 코어는 Rust로 작성되며, macOS와 Windows는 WebView, Chromium, 상주 JavaScript runtime 대신 네이티브 UI, 네이티브 입력 시스템, GPU 렌더링을 사용합니다.

## 주요 특징

- **Markdown-first** — Markdown 텍스트 자체가 문서 모델이며 리치 텍스트 모델과 왕복 직렬화를 하지 않습니다.
- **렌더링 상태에서 직접 편집** — 실시간 Source Projection과 Decoration으로 보이는 내용을 그대로 편집할 수 있습니다.
- **네이티브 데스크톱 경험** — macOS는 Swift/AppKit + Metal, Windows는 Win32 + DirectWrite/D3D11 + TSF/IME를 사용합니다.
- **증분 고성능 코어** — 구문, 레이아웃, scene, 렌더링은 Revision에 연결되며 변경되거나 현재 보이는 영역만 다시 계산합니다.
- **국제 문자 입력 우선** — CJK, emoji, 결합 문자, bidi/RTL, 네이티브 IME composition을 핵심 요구사항으로 다룹니다.
- **로컬 우선** — Markdown 문서는 일반 로컬 파일로 유지되며 외부 변경과 저장 충돌을 처리합니다.
- **Markdown 워크플로** — 코드 블록, 표, 수식, 다이어그램, 이미지, 클립보드/내보내기, 시스템 인쇄를 Markdown 원본 모델 위에서 처리합니다.
- **오픈소스** — Apache-2.0 라이선스.

## 다운로드

현재 macOS 및 Windows 빌드는 **[GitHub Releases](https://github.com/xiaodou997/yu/releases/latest)** 에서 받을 수 있습니다.

| 플랫폼 | 현재 지원 |
| --- | --- |
| macOS | Apple Silicon, macOS 26 이상. 정식 빌드는 Developer ID 서명 및 Apple 공증을 거칩니다. |
| Windows | x64, Windows 10 2004 이상. 설치형과 포터블 빌드를 제공합니다. 현재 릴리스는 Authenticode 미서명 상태입니다. |
| Linux | 공유 Rust 코어는 검증하고 있지만 데스크톱 패키지는 아직 배포하지 않습니다. |

## 동작 방식

Yu는 WebView Markdown 편집기, HTML 편집기, 또는 “리치 텍스트 + Markdown 가져오기/내보내기” 구조가 아닙니다. 하나의 문서 원본과 하나의 렌더링 경로를 유지합니다.

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

핵심 불변 조건:

1. Markdown source는 항상 유일한 원본입니다.
2. 영구 변경은 모두 Transaction을 거칩니다.
3. Markdown 의미론은 `yu-markdown`에만 존재하고 시각 상태는 Decoration으로 표현합니다.
4. 파생 데이터는 Revision에 연결되며 오래된 결과는 폐기합니다.
5. IME composition은 transient overlay이며 문서 상태를 오염시키지 않습니다.
6. 플랫폼 계층은 Markdown을 파싱하지 않습니다.
7. 캐시, 비동기 리소스, GPU 상태는 편집 의미론을 바꿀 수 없습니다.
8. 두 번째 렌더링 경로는 없습니다.

## 플랫폼 아키텍처

- **공유 코어**: Rust crates가 텍스트 저장, 구문, 편집기 상태, Decoration, 레이아웃, Scene, RenderPlan, 리소스, 저장소, 워크스페이스, 내보내기를 담당합니다.
- **macOS**: Swift/AppKit 네이티브 셸, CoreText, Metal, FSEvents, 네이티브 메뉴, 입력, Accessibility, 창 라이프사이클.
- **Windows**: Rust/windows-rs + Win32 네이티브 셸, DirectWrite, D3D11, TSF/IME, DPI/테마, 네이티브 파일 라이프사이클.
- **Linux**: 현재는 공유 코어 검증이 중심입니다.

## 저장소 구조

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

## 개발

Rust 1.98.1을 사용합니다.

```bash
git clone https://github.com/xiaodou997/yu.git
cd yu

tools/verify.sh
tools/verify.sh --rust-only

# macOS
platform/macos/yu-shell-macos/run-app.sh README.md
```

Windows:

```powershell
./platform/windows/yu-shell-windows/run-self-checks.ps1
```

macOS FFI 경계를 변경했다면 SwiftPM의 오래된 증분 산출물을 피하기 위해 clean build를 사용하세요.

```bash
platform/macos/yu-shell-macos/run-self-checks.sh --clean-build
```

## 지원 및 개인정보 보호

- 버그 신고 및 기능 제안: [GitHub Issues](https://github.com/xiaodou997/yu/issues)
- 최신 릴리스: [GitHub Releases](https://github.com/xiaodou997/yu/releases/latest)
- macOS 개인정보 보호: [PRIVACY.md](PRIVACY.md)
- Windows 개인정보 보호: [PRIVACY-WINDOWS.md](PRIVACY-WINDOWS.md)

## 기여

Issue와 Pull Request를 환영합니다. 편집기 동작을 변경할 때는 동작 테스트를 추가하고, 증분 알고리즘은 전체 알고리즘과의 동등성을 확인해야 합니다. 플랫폼 또는 공유 코어 변경을 제출하기 전에 로컬 검증을 실행해 주세요.

## License

Yu Markdown은 [Apache License 2.0](LICENSE)에 따라 오픈소스로 공개됩니다.
