<p align="center">
  <img src="./platform/macos/yu-shell-macos/Assets/branding/yu-logo-master-1024.png" width="160" alt="Yu Markdown Logo" />
</p>

<h1 align="center">Yu Markdown</h1>

<p align="center">
  <strong>一個由 Rust 驅動的原生、Markdown-first 所見即所得編輯器。</strong>
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
  <a href="https://github.com/xiaodou997/yu/blob/main/LICENSE">
    <img src="https://img.shields.io/badge/License-Apache--2.0-green?style=flat-square" alt="Apache-2.0" />
  </a>
</p>

Yu Markdown 始終把 **Markdown 原始碼作為唯一真源**，同時提供接近完成文件的直接編輯體驗。共享編輯器核心使用 Rust；macOS 與 Windows 使用各自的原生產品殼、原生輸入系統與 GPU 渲染，不依賴 WebView、Chromium 或常駐 JavaScript runtime。

## 核心亮點

- **Markdown-first**：Markdown 原始文字就是文件模型，不經過富文字模型來回序列化。
- **渲染態直接編輯**：透過即時 Source Projection 與 Decoration，讓渲染後的內容仍可直接編輯。
- **原生桌面體驗**：macOS 使用 Swift/AppKit + Metal；Windows 使用 Win32 + DirectWrite/D3D11 + TSF/IME。
- **增量高效能核心**：語法、版面配置、場景與渲染都綁定 Revision，只重算變更或目前可見的內容。
- **國際化輸入優先**：中日韓文字、emoji、組合字元、雙向/RTL 文字與原生 IME 都是一等公民。
- **本機優先**：Markdown 文件始終是電腦上的一般檔案，並提供外部變更偵測與儲存衝突處理。
- **完整 Markdown 工作流**：程式碼區塊、表格、數學公式、圖表、圖片、剪貼簿/匯出與系統列印都圍繞 Markdown 真源運作。
- **開源**：採用 Apache-2.0 授權。

## 下載

目前 macOS 與 Windows 正式版本統一從 **[GitHub Releases](https://github.com/xiaodou997/yu/releases/latest)** 下載。

| 平台 | 目前支援 |
| --- | --- |
| macOS | Apple Silicon，macOS 26 或更新版本。正式包使用 Developer ID 簽署並通過 Apple 公證。 |
| Windows | x64，Windows 10 2004 或更新版本。提供安裝版與可攜版；目前版本尚未進行 Authenticode 簽署。 |
| Linux | 已驗證共享 Rust 核心，暫未發布桌面安裝包。 |

## 運作方式

Yu 不是 WebView Markdown 編輯器，也不是 HTML 編輯器或「富文字編輯器 + Markdown 匯入匯出」。整個系統維持一個文件真源與一條渲染路徑：

```text
Markdown 原始碼
    ↓
Transaction + Revision Snapshot
    ↓
增量 Markdown 語法
    ↓
Source Projection + Decoration
    ↓
原生版面配置
    ↓
Retained Scene
    ↓
GPU Renderer
```

核心約束：

1. Markdown source 永遠是唯一真源。
2. 所有永久修改都經過 Transaction。
3. Markdown 語義只存在於 `yu-markdown`；視覺狀態由 Decoration 表達。
4. 所有衍生資料都綁定 Revision，過期結果整體拒絕。
5. IME composition 始終是 transient overlay，不污染文件狀態。
6. 平台層不解析 Markdown。
7. 快取、非同步資源與 GPU 狀態不能改變編輯語義。
8. 不存在第二條渲染路徑。

## 平台架構

- **共享核心**：Rust crates 負責文字儲存、語法、編輯狀態、Decoration、版面配置、Scene、RenderPlan、資源、儲存、工作區與匯出。
- **macOS**：Swift/AppKit 原生產品殼，CoreText、Metal、FSEvents、原生選單、輸入法、Accessibility 與視窗生命週期。
- **Windows**：Rust/windows-rs + Win32 原生產品殼，DirectWrite、D3D11、TSF/IME、DPI/主題與原生檔案生命週期。
- **Linux**：目前主要驗證共享核心。

## 儲存庫結構

```text
crates/yu-core          座標、Revision、Anchor
crates/yu-text          Rope、Snapshot、Transaction
crates/yu-syntax        增量 CST
crates/yu-markdown      Markdown 語義與擴充
crates/yu-state         EditorState、History、Facet
crates/yu-decoration    source ↔ visual 映射
crates/yu-layout        行/元件版面配置、bidi、hit-test
crates/yu-scene         retained scene 與 damage tracking
crates/yu-render        後端中立 RenderPlan
crates/yu-assets        圖片/資源排程與快取
crates/yu-storage       Markdown 工作階段、原子儲存、檔案監看
crates/yu-workspace     tab 與 document session 生命週期
crates/yu-export        revision-bound 剪貼簿與 HTML 匯出

platform/macos          macOS 原生 shell / font / render / storage 適配
platform/windows        Windows 原生 shell / 打包 / 安裝器
tools                   驗證、檢查、基準與發布工具
```

## 開發

專案固定使用 Rust 1.98.1。

```bash
git clone https://github.com/xiaodou997/yu.git
cd yu

tools/verify.sh
tools/verify.sh --rust-only

# macOS：建置並啟動原生應用程式
platform/macos/yu-shell-macos/run-app.sh README.md
```

Windows：

```powershell
./platform/windows/yu-shell-windows/run-self-checks.ps1
```

如果修改了 macOS FFI 邊界，請使用乾淨建置以避免 SwiftPM 增量快取造成假通過：

```bash
platform/macos/yu-shell-macos/run-self-checks.sh --clean-build
```

## 支援與隱私

- 問題回報與功能建議：[GitHub Issues](https://github.com/xiaodou997/yu/issues)
- 最新版本：[GitHub Releases](https://github.com/xiaodou997/yu/releases/latest)
- macOS 隱私說明：[PRIVACY.md](PRIVACY.md)
- Windows 隱私說明：[PRIVACY-WINDOWS.md](PRIVACY-WINDOWS.md)

## 貢獻

歡迎提交 Issue 與 Pull Request。修改編輯行為時應提供行為測試；修改增量演算法時應驗證結果與完整演算法等價。提交平台層或共享核心改動前，請先執行本機驗證。

## License

Yu Markdown 依 [Apache License 2.0](LICENSE) 開源。
