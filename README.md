# 羽 / Yu Editor

[![CI](https://github.com/xiaodou997/yu/actions/workflows/ci.yml/badge.svg)](https://github.com/xiaodou997/yu/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Yu Editor 是一个开源、原生、Markdown-first 的桌面编辑器项目。项目以 Markdown 源码为
唯一持久化真源，通过增量语法、实时 Source Projection、自研编辑模型和 GPU 渲染，在
macOS、Windows 与 Linux 上提供低资源、低延迟的编辑体验。

macOS 是第一个产品级平台。共享编辑器内核使用 Rust；平台输入、窗口、Accessibility 等
能力允许使用 Swift、Objective-C 或其他适合该平台的语言实现。

> **发布状态：** [v0.1.3](https://github.com/xiaodou997/yu/releases/tag/v0.1.3) 已发布。
> 下载：[macOS Apple Silicon](https://github.com/xiaodou997/yu/releases/download/v0.1.3/Yu-0.1.3-4-arm64.dmg) · [Windows x64 安装包](https://github.com/xiaodou997/yu/releases/download/v0.1.3/Yu-0.1.3-windows-x64-setup.exe) · [Windows 便携版](https://github.com/xiaodou997/yu/releases/download/v0.1.3/Yu-0.1.3-windows-x64.zip)。
> macOS 需要 26 或更新版本，已签名并经 Apple 公证；Windows 需要 Windows 10 2004 或更新版本，当前未签名。

## 支持与隐私

- 使用问题或缺陷：[GitHub Issues](https://github.com/xiaodou997/yu/issues)
- 隐私说明：[PRIVACY.md](PRIVACY.md)

## 设计目标

Yu 的技术本质是：

> 一个 Markdown 源码编辑器内核 + 一个实时 Source Projection 系统
> + 一个增量原生布局引擎 + 一个 retained GPU renderer
> + 少量平台原生输入与窗口适配。

它**不是** WebView Markdown 编辑器、富文本编辑器、HTML 编辑器或 Markdown 格式转换器。

核心设计原则：

1. Markdown source 永远是唯一真源，不通过富文本模型往返序列化；
2. 所有永久修改都经过 Transaction；
3. 视觉表现的唯一来源是 Decoration，Markdown 语义只存在于 `yu-markdown` 一个 crate；
4. 所有派生数据都绑定 Revision，过期结果整体拒绝；
5. IME composition 永远是 transient overlay；
6. 平台层不解析 Markdown；
7. 缓存、GPU 和异步资源不能改变编辑语义；
8. 只处理发生变化和当前可见的内容；
9. 不存在第二条渲染路径——Rust 渲染器是唯一渲染器；
10. crate 依赖图必须是严格 DAG，由 CI 强制。

中文、日文、RTL、emoji、组合字符与原生 IME 是一等公民。不依赖 Chromium、DOM
或常驻 JavaScript runtime。

## 平台状态

macOS 使用 Swift 原生产品壳与 Metal 渲染；Windows 使用 Win32 原生产品壳、
DirectWrite/D3D11 和 TSF/IME 输入。两端共享 Rust 编辑器内核。
Linux 目前只验证共享内核，尚未提供桌面安装包。

## 仓库结构

目标形态。依赖方向严格单向，反向依赖是 CI 失败。

```text
crates/yu-core          坐标、Revision、Anchor
crates/yu-text          Rope（ropey）、Snapshot、Transaction 原语
crates/yu-syntax        增量 CST：block/inline 两级、fragment 复用、精确 range（S3 已建立）
crates/yu-markdown      ★ Markdown 语法 extension 集合（Markdown 只存在于这一层）
crates/yu-state         EditorState、Transaction 应用、History、Facet
crates/yu-decoration    ★ RangeSet<Decoration>、source↔visual 映射
crates/yu-layout        行盒、widget 盒、UAX#14 断行、UAX#9 bidi、hit-test
crates/yu-scene         retained primitives 与 damage 追踪
crates/yu-render        后端中立 RenderPlan：FillRect / RoundedFillRect / Glyph / Image / EmbeddedSvg（变体集合冻结，见 invariants.md E3）
crates/yu-font          字体解析、shaping、栅格化契约（只依赖 yu-core）
crates/yu-assets        图片/嵌入资源的异步调度、LRU 与内存预算
crates/yu-storage       UTF-8 Markdown 文档会话、原子保存、外部变更检测
crates/yu-workspace     tab 与 document session 生命周期
crates/yu-export        Revision-bound 剪贴板与 HTML 导出（comrak）
platform/macos/yu-font-macos    CoreText 字体目录、fallback、shaping、栅格化
platform/macos/yu-render-macos  Metal device、CAMetalLayer、render plan 编码
platform/macos/yu-storage-macos FSEvents 文件通知适配
platform/macos/yu-shell-macos   Swift 产品壳：NSWindow / 菜单 /
                                NSTextInputClient / Accessibility
platform/windows/yu-font-windows DirectWrite analysis/fallback/shaping/rasterization
platform/windows/yu-render-windows D3D11 device、DXGI swapchain、RenderPlan 命令消费
platform/windows/yu-shell-windows Rust + windows-rs 产品壳：Win32 窗口 / 菜单 /
                                  DPI / 主题 / 文件生命周期 / D3D surface host
tools/yu-inspect        Markdown 结构检查 CLI
tools/yu-bench          可重复的参考 workload
```

旁路依赖（不在主链路上）：`tree-sitter` 仅用于 fenced code block 内部的代码高亮；
`comrak` 仅用于 HTML 导出与 CommonMark spec 差分测试。

## 获取源码

```bash
git clone git@github.com:xiaodou997/yu.git
cd yu
```

项目固定使用 Rust 1.98.1。构建 macOS 产品壳还需要 Xcode/Swift 工具链。

## 本地验证

```bash
# 全量验证：fmt / clippy / test / FFI 头文件一致性 / 产品壳 self-check
tools/verify.sh
tools/verify.sh --rust-only     # 只跑 Rust 检查
tools/verify.sh --clean         # 产品壳用干净构建（改动 FFI 边界后必须）

# 构建并（重新）启动 Yu.app
platform/macos/yu-shell-macos/run-app.sh README.md

# Windows 真机/runner 产品壳自检
./platform/windows/yu-shell-windows/run-self-checks.ps1

# 其它
cargo test -p yu-render-macos -- --ignored       # 需要有 Metal device 的 macOS session
cargo run -p yu-inspect -- README.md
cargo run --release -p yu-bench -- --size-mib 1 --iterations 20 --random-edits 2000 --retained-snapshots 8
```

`run-app.sh` 会先终止已在运行的实例：macOS 的 `open` 对运行中的 app 只会把它
带到前台、不会加载新二进制，直接 `open` 会让已修好的 bug 看起来仍在复现。

`verify.sh` 存在的理由是手敲验证命令容易漏。例如
`cargo test --workspace | grep "^test result: ok" | awk '{s+=$4}'` 会跳过
`test result: FAILED` 的行——失败被静默吞掉，还显示出一个看起来正常的用例数。
脚本以退出码为准，任一步失败立即中止。

Swift 产品壳通过 `YuStorageFFI` C module 链接 Rust static library，因此必须先运行
`build-rust-ffi.sh`，或使用会自动执行它的 `build-app.sh` / `run-self-checks.sh --build`。
构建产物位于被忽略的 `.rust/` 与 `.build/`，不会提交到仓库。

> **改动 FFI 边界后请用 `run-self-checks.sh --clean-build`。** SwiftPM 的增量构建
> 可能不会重编引用已删类型的文件，本地看到「构建通过」而 CI 的干净检出会失败。

## Windows 安装包构建

在 Windows x64 电脑上安装 Git、Rust（MSVC 工具链）、Visual Studio Build Tools
的 C++ 桌面开发组件与 Windows SDK，以及 Inno Setup 6 或 7。使用干净的 Git
检出，在仓库根目录运行：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tools\release-windows.ps1
```

脚本完成 Rust 检查、原生窗口与辅助功能自检，再生成 Release 构建并验证安装和卸载。
也可在 GitHub Actions 手动运行 `Windows installer` 工作流，成功后下载
`Yu-windows-x64-提交号` artifact，无需自备 Windows 构建电脑。

本地构建成功后会打印安装包完整路径，产物位于 `artifacts/releases/windows-时间戳/`：

- `Yu-版本-windows-x64-setup.exe`：可直接运行的安装包；
- `Yu-版本-windows-x64.zip`：便携版本；
- `SHA256SUMS.txt`：文件校验和。

安装器不需要管理员权限。当前 Windows 分发未签名，可能出现未知发布者提示。
脚本任一步失败都停止，不会把失败产物标记成可发布版本。

## 贡献

欢迎通过 GitHub Issues 报告问题。修改编辑行为时应提供行为测试；修改增量算法时
应验证结果与完整算法等价。提交前请运行本地验证，并检查对应平台的 CI 结果。

`docs/` 和 `.notes/` 用于本地开发资料，不纳入 Git 跟踪。

## License

Yu Editor 使用 [Apache License 2.0](LICENSE) 发布。该许可证允许使用、修改、分发和商业
集成，并包含明确的专利授权；分发时需要保留许可证及适用的版权和归属声明。
