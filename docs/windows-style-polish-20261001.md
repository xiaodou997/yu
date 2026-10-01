# Windows 原生界面样式优化记录

日期：2026-10-01。用户确认微软拼音基础第一轮提交、取消、撤销和重做正常，
要求继续改善界面样式。本记录随 Windows 原生输入修复和界面优化提交维护；
软件门禁与用户人工验收结果分别记录，完整第四组仍待验收。

## 界面变化

- 复用共享 `Appearance::YuLight/YuDark`：正文颜色、标题比例、行距与阅读列宽度
  跟随现有 Yu 主题，不在 Windows 另建 Markdown 样式或解析器。
- 侧栏采用浅灰背景、圆角分段导航、柔和的当前项选中状态，统一内边距与 28 DIP 列表行高。
- UI 控件按界面语言选用 Microsoft YaHei UI / Yu Gothic UI / Malgun Gothic / Segoe UI；
  导航为 13 DIP 半粗，内容为 13 DIP，辅助文字与状态栏为 11 DIP，随 DPI 缩放。
  正文继续使用既有 DirectWrite Segoe UI 与字体 fallback 链。
- 搜索框移除原生凹陷边框，使用细线圆角框，保留原生 EDIT 的键盘和 IME 语义。
- 底部状态栏为 24 DIP，左侧显示就绪状态，右侧显示缩放、编码和 Markdown。
- 正文两侧留白默认 24 DIP、顶部 32 DIP，宽窗口中限制为 808 DIP 阅读列；
  小窗口自动收窄。侧栏宽度上限 240 DIP，同时限制在窗口宽度的 35%。

正文留白通过真实编辑 surface HWND 的位置实现；TSF 屏幕坐标、鼠标命中和渲染
仍以同一个 HWND 为基准。背景 canvas 保持在 GPU surface 下方，避免遮住正文。
实际截图检查发现并修正了新增背景与后创建的 surface 的层级问题，回归检查了顶层子窗口。
字体和 GDI 区域资源按原生对象生命周期释放。

## 已执行验证

| 项目 | 结果 |
| --- | --- |
| yu-font-windows / yu-render-windows / yu-shell-windows | 19/19、2/2、19/19 PASS |
| HWND + DirectWrite + D3D + TSF 初始化与 clean close | PASS |
| 字体角色和行高 96→192→96 DPI 回归 | PASS |
| 真实 HWND 页边距、caret 屏幕坐标、ACP 点命中、GPU surface 层级 | PASS |
| 当前 192 DPI 可见窗口文件、大纲、Unicode 搜索与文件打开 | 定向 Win32 消息及实际截图 PASS |
| 三个 Windows crate all-targets clippy -D warnings | PASS |
| cargo fmt --all -- --check / git diff --check | PASS |

本轮未修改 yu-editor，其前一轮完整回归为 573 passed / 0 failed / 1 ignored。
本轮控件 DPI 回归不等于真实系统缩放切换；深色控件回归不等于完整系统深色人工验收。
之前微软拼音人工结果记录在 [真机验收计划](windows-group4-manual-acceptance.md)。
新版移动了正文 HWND，需要复查候选框跟随和基础输入；其余 IME、物理键鼠、
单屏实际 DPI 切换和跨屏 DPI 仍按原验收清单推进。彩色 emoji 仍属于第五组。

## 运行与证据

证据目录：`artifacts/windows-group4/20261001-style/`（Git 忽略）。
包含 native-self-check.log、clippy.log、visible-results.json、四个面板/正文截图、
工作副本、独立 exe、哈希与 PID。初步探查截图保留，仅最终截图作为可见验收证据。

最终 exe：`yu-shell-windows-a9124ce3.exe`。
SHA256：`A9124CE31831078F0AA24DCC61318939CBDF9A8EAE1E9445D7C9D88432010DC3`。
该独立副本可在编译时继续运行。用户原输入测试窗口 PID 12404 保留，包含未保存修改。
新预览使用独立测试文件，避免修改原窗口中的内容；测试文件和生成物不是正式产品资源。
