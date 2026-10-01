# Windows 侧栏、状态栏与滚动修复：2026-10-01

测试基线为 `main@83e1bdb27740f8cc623622117316fd596332702a` 加本轮原生修复。
基础界面和原生软件门禁通过，真实 IME、物理键鼠和其他缩放比例仍待验收。

## 本轮修复

- 移除单个 STATIC 控件中的三个占位标签，改为可操作的文件、大纲、搜索导航。
- 通过 `SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS)` 获取系统 message font，
  设置至少 13 DIP 的 UI 字号；DPI 或系统字体设置变化后替换所有控件字体，随后释放旧字体。
- 增加内边距、32 DIP 列表行高、28 DIP 状态栏、分隔线与浅色/深色颜色。
  导航保留 native button 键盘语义，并绘制选中、悬停和焦点状态。
- 文件页列出当前文档同目录的 `.md` / `.markdown` 文件，单击选择，双击或 Enter 打开，
  切换文档继续使用共享保存/丢弃/取消流程。未命名文档显示打开文件的提示。
- 大纲使用共享 `OutlineTree` 提供的标签、身份、层级及源码范围；搜索使用共享
  `SearchResults` 提供的上下文标签和 UTF-8 命中范围。平台没有另写 Markdown 解析或搜索算法。
- 搜索查询和源 revision 变化后更新结果，防止编辑、撤销后继续使用旧范围。
  Ctrl+F 聚焦搜索，Enter 跳转，Escape 返回正文；输入框的 Ctrl+Z/Y 不再经过正文 accelerator。
  搜索框原生 IME composition 活跃时，Enter/Escape 留给 EDIT 的输入法处理。
- 渲染完成后通过共享 `adopt_layout_snapshot` 接收同版布局测量；导航用共享 caret 滚动请求，
  并在新可见块测量后检查目标 caret 已可见。
- scene viewport 使用 `(0, scroll_y, width, height)`：共享 draw-command builder 会减去该原点。
  原代码只改变待绘制块却保持 scene 原点为零，导致滚动后绘制落到窗口外，正文变空白。
- 初始化绘制不向 `DrawTextW` 传空切片；修复实际启动时的 user32 回调异常。
  侧栏背景放到 sibling 最底层，避免背景覆盖按钮和列表。

字体 API 依据：[Microsoft SystemParametersInfoForDpi](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfofordpi)。

没有新增第三方 crate；仅启用已有 windows-rs 的 `Win32_UI_Controls` 功能。
GDI 只绘制产品控件，正文继续走共享 Rust scene / D3D。

## 已执行验证

| 项目 | 结果 |
| --- | --- |
| yu-font-windows | 19/19 PASS |
| yu-render-windows | 2/2 PASS |
| yu-shell-windows | 19/19 PASS |
| 原生 cargo check / PE 链接 / 真实 HWND + DirectWrite + D3D + TSF clean close | PASS |
| Windows 三个 crate all-targets clippy -D warnings | PASS |
| cargo fmt --all -- --check / git diff --check | PASS |
| UI 字体96→192→96、列表行高、浅色/深色切换 | 原生控件与渲染回归 PASS；不等于系统缩放人工切换 |
| 共享大纲标签、中文/emoji 搜索范围、撤销后的结果刷新 | 原生回归 PASS |
| 真实窗口下远处搜索目标的布局、scene 可见选区、source/revision 不变 | 原生集成回归 PASS |
| 当前 192 DPI（200%）文件、大纲、搜索页面及文件打开 | 定向 Win32 消息与实际截图已检查 |
| 微软拼音候选提交、Escape 取消、Undo/Redo 基础第一轮 | 用户人工确认 PASS；不等于完整 IME 清单通过 |
| 微软五笔/日文/韩文与完整 composition/candidate 清单 | 待执行 |

yu-editor 全量回归在前一轮为 573 passed / 0 failed / 1 ignored，本轮未修改该 crate。
Python 依赖方向检查仍未重跑：本机未找到可用解释器；本轮未改变 crate 依赖方向。

本机证据目录（Git 忽略）：`artifacts/windows-group4/20261001-chrome/`。
包含 native-self-check.log、clippy.log、visible-results.json、files-window.png、
outline-window.png、search-window.png、unicode-window-final.png 和测试工作副本。
定向消息检查验证文件列表、7个标题、2处 Unicode 匹配、目标选择和文件打开，
不能代替用户的物理按键与真实 IME。

最终 exe SHA256：`3C40C048AA28CE335752620784ABD385D08C9C4BDB59A73604DB6AB6166B3600`。
为避免编译时关闭用户正在操作的窗口，当前人工测试启动独立副本
`artifacts/windows-group4/20261001-chrome/yu-shell-windows-3c40c048.exe`。
12:25 启动的测试 PID 为 12404，标题为 `unicode-work.md — Yu`；此后不主动关闭。

## 疑似拼音退出报告

用户在本轮重建期间报告“输入拼音时好像闪退”。此前代理为解除 exe 编译锁主动关闭过
标题仍为 clean 的测试窗口；未提交 composition 不会让文档 dirty，不能据此认定无人操作。
12:24 检查 Application 事件，最近的 Yu 异常为 11:59 启动绘制异常，未找到对应此次输入的新事件。
这支持需要区分主动关闭与异常退出，但不能证明此次真实 IME 没有问题。
已重新打开独立副本。用户随后确认微软拼音第一轮提交“中文”、取消 `ceshi`、
Ctrl+Z/Ctrl+Y 均正常；本轮未复现退出。此前退出原因仍未确认，不能记为已定位的 IME 崩溃修复。

## 范围与后续

本轮是基础面板接线：文件页为同目录 Markdown 列表，大纲为层级缩进列表。
完整文件夹树、大纲折叠及 macOS 面板功能对齐另需产品验收。
微软拼音基础第一轮已通过；随后样式优化见 [样式优化记录](windows-style-polish-20261001.md)。
新版复查候选几何与基础提交/取消/Undo/Redo 后，再推进其他 IME、物理键鼠、
单屏实际 DPI 与主题切换；跨屏 DPI 仍因只有一块显示器待补。
彩色 emoji 和图片/公式/Mermaid 资源链仍属于第五组，不能用本轮侧栏修复代替。
