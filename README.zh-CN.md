<p align="center">
  <img src="./platform/macos/yu-shell-macos/Assets/branding/yu-logo-master-1024.png" width="160" alt="Yu Markdown Logo" />
</p>

<h1 align="center">Yu Markdown</h1>

<p align="center">
  <strong>一个由 Rust 驱动的原生、Markdown-first 所见即所得编辑器。</strong>
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

Yu Markdown 始终把 **Markdown 源码作为唯一真源**，同时提供接近成品文档的直接编辑体验。共享编辑器内核使用 Rust；macOS 与 Windows 使用各自的原生产品壳、原生输入系统和 GPU 渲染，不依赖 WebView、Chromium 或常驻 JavaScript runtime。

## 核心亮点

- **Markdown-first**：Markdown 源文本就是文档模型，不经过富文本模型来回序列化。
- **渲染态直接编辑**：通过实时 Source Projection 与 Decoration，让渲染后的内容仍然可以直接编辑。
- **原生桌面体验**：macOS 使用 Swift/AppKit + Metal；Windows 使用 Win32 + DirectWrite/D3D11 + TSF/IME。
- **增量高性能内核**：语法、布局、场景和渲染都绑定 Revision，只重算发生变化或当前可见的内容。
- **国际化输入优先**：中文、日文、韩文、emoji、组合字符、双向/RTL 文本和原生 IME 都是一等公民。
- **本地优先**：Markdown 文档始终是你电脑上的普通文件，并提供外部修改检测与保存冲突处理。
- **完整 Markdown 工作流**：代码块、表格、数学公式、图表、图片、剪贴板/导出和系统打印都围绕 Markdown 真源工作。
- **开源**：使用 Apache-2.0 许可证。

## 下载

当前 macOS 与 Windows 正式版本统一从 **[GitHub Releases](https://github.com/xiaodou997/yu/releases/latest)** 下载。

| 平台 | 当前支持 |
| --- | --- |
| macOS | Apple Silicon，macOS 26 或更新版本。正式包使用 Developer ID 签名并经 Apple 公证。 |
| Windows | x64，Windows 10 2004 或更新版本。提供安装版与便携版；当前版本暂未进行 Authenticode 签名。 |
| Linux | 已验证共享 Rust 内核，暂未发布桌面安装包。 |

## 它是怎么工作的

Yu 不是 WebView Markdown 编辑器，也不是 HTML 编辑器或「富文本编辑器 + Markdown 导入导出」。整个系统坚持一个文档真源和一条渲染路径：

```text
Markdown 源码
    ↓
Transaction + Revision Snapshot
    ↓
增量 Markdown 语法
    ↓
Source Projection + Decoration
    ↓
原生布局
    ↓
Retained Scene
    ↓
GPU Renderer
```

核心约束：

1. Markdown source 永远是唯一真源。
2. 所有永久修改都经过 Transaction。
3. Markdown 语义只存在于 `yu-markdown`；视觉状态由 Decoration 表达。
4. 所有派生数据都绑定 Revision，过期结果整体拒绝。
5. IME composition 始终是 transient overlay，不污染文档状态。
6. 平台层不解析 Markdown。
7. 缓存、异步资源和 GPU 状态不能改变编辑语义。
8. 不存在第二条渲染路径。

## 平台架构

- **共享内核**：Rust crates 负责文本存储、语法、编辑状态、Decoration、布局、Scene、RenderPlan、资源、存储、工作区与导出。
- **macOS**：Swift/AppKit 原生产品壳，CoreText、Metal、FSEvents、原生菜单、输入法、Accessibility 与窗口生命周期。
- **Windows**：Rust/windows-rs + Win32 原生产品壳，DirectWrite、D3D11、TSF/IME、DPI/主题与原生文件生命周期。
- **Linux**：当前主要验证共享内核。

## 仓库结构

```text
crates/yu-core          坐标、Revision、Anchor
crates/yu-text          Rope、Snapshot、Transaction
crates/yu-syntax        增量 CST
crates/yu-markdown      Markdown 语义与扩展
crates/yu-state         EditorState、History、Facet
crates/yu-decoration    source ↔ visual 映射
crates/yu-layout        行/组件布局、bidi、hit-test
crates/yu-scene         retained scene 与 damage tracking
crates/yu-render        后端中立 RenderPlan
crates/yu-assets        图片/资源调度与缓存
crates/yu-storage       Markdown 会话、原子保存、文件监听
crates/yu-workspace     tab 与 document session 生命周期
crates/yu-export        revision-bound 剪贴板与 HTML 导出

platform/macos          macOS 原生 shell / font / render / storage 适配
platform/windows        Windows 原生 shell / 打包 / 安装器
tools                   验证、检查、基准与发布工具
```

## 开发

项目固定使用 Rust 1.98.1。

```bash
git clone https://github.com/xiaodou997/yu.git
cd yu

# 全量验证
tools/verify.sh

# 只跑 Rust
tools/verify.sh --rust-only

# macOS：构建并启动原生应用
platform/macos/yu-shell-macos/run-app.sh README.md
```

Windows：

```powershell
./platform/windows/yu-shell-windows/run-self-checks.ps1
```

如果修改了 macOS FFI 边界，请使用干净构建，避免 SwiftPM 增量缓存造成假通过：

```bash
platform/macos/yu-shell-macos/run-self-checks.sh --clean-build
```

## 支持与隐私

- 问题反馈与功能建议：[GitHub Issues](https://github.com/xiaodou997/yu/issues)
- 最新版本：[GitHub Releases](https://github.com/xiaodou997/yu/releases/latest)
- macOS 隐私说明：[PRIVACY.md](PRIVACY.md)
- Windows 隐私说明：[PRIVACY-WINDOWS.md](PRIVACY-WINDOWS.md)

## 贡献

欢迎提交 Issue 与 Pull Request。修改编辑行为时应提供行为测试；修改增量算法时应验证结果与完整算法等价。提交平台层或共享内核改动前，请先运行本地验证。

## License

Yu Markdown 基于 [Apache License 2.0](LICENSE) 开源。
