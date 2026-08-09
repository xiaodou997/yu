# macOS Text Input Spike

这是可丢弃的 AppKit 风险实验，不是 Yu 产品 UI。它用于验证自绘 `NSView` 在不继承
`NSTextView` 的情况下接入 `NSTextInputClient`：

- marked/preedit text；
- commit 与 cancel；
- UTF-16 selection range；
- candidate window rect；
- 基本 grapheme 删除和移动；
- TextKit shaping 与点击位置查询；
- `NSAccessibility` text area、UTF-16 range 与 screen bounds 查询。

构建：

```bash
swift build --package-path experiments/macos-text-input
```

生成可由 LaunchServices 和 Accessibility 识别的临时 `.app`：

```bash
experiments/macos-text-input/build-app.sh
open experiments/macos-text-input/.build/YuMacTextInputSpike.app
```

手工运行：

```bash
swift run --package-path experiments/macos-text-input YuMacTextInputSpike
```

运行后切换中文拼音、日文或其他输入法。终端会记录 `setMarkedText`、`insertText` 和
`unmarkText` 事件。需要确认：

1. 拼音更新只改变 marked range；
2. 选词后只产生一次 commit；
3. candidate window 跟随 caret；
4. Escape 取消后正文恢复正确；
5. emoji 和组合字符退格时不会被拆成无效序列。

启动后程序还会运行两级 Accessibility 检查：先直接校验 View 的文本、行范围和 caret frame，
再通过系统 `AXUIElement` 查询 focused element 的 role、字符数、首行文本及 bounds。成功时终端
应出现 `AX self-check` 和 `AX runtime probe trusted=true role=AXTextArea`。后者依赖当前终端或
生成的 `.app` 已获 macOS Accessibility 权限。

该实验暂时直接保存 UTF-16 selection，因为 AppKit 协议使用 `NSRange`。接入 Rust 时必须由
平台适配层转换成带 Revision 的 `TextAnchor`，正式 composition 则进入临时 Overlay，不能在
每次 `setMarkedText` 时提交 Undo Transaction。

产品实现还必须把 Accessibility 文本查询绑定到 Rust `Revision`。屏幕 bounds 由当前 Layout
回答；存在 composition 时，AX、`NSTextInputClient` 和绘制必须看到同一份 overlay 状态。
