# ADR 0055：macOS Spike 真实 `NSScrollView` Host Attachment

## 状态

已接受（Phase 1）

## 背景

ADR 0054 只验证了一个无窗口 `NSScrollView` 能消费 Rust 的 absolute caret scroll
request。下一步需要证明同一个边界可以承载真实 `TextInputView`，否则命令、IME commit 和
Accessibility selection 改变后，caret reveal 仍然只是孤立的协议测试。

macOS spike 的 TextKit 使用 native point 作为绘制和 `NSClipView` 坐标，而 Rust viewport
实验默认使用 line-height 为 1 的逻辑单位。让任一侧直接假设另一侧的单位相同，会把当前
风险实验的尺寸差异错误地固化成产品契约。

## 决策

- `TextInputView` 作为 `NSScrollView.documentView`，`YuNativeViewportAdapter` 仍只拥有
  `NSScrollView` 的 viewport 状态；Rust 不接触 AppKit 对象。
- adapter 在每次同步时维护当前 Revision 和 TextKit `usedRect` 推导的 native content
  height，并从 `NSClipView.bounds` 读取当前 native `scroll_y` 与 viewport height。
- `RustCompositionBridge.caretScrollRequest` 接受一个正的 `scale`：查询 Rust 前将 native
  scroll/viewport/margin 除以 scale，返回请求时把 caret geometry、current/target scroll 和
  margin 乘回 native 单位。当前 spike 用 TextKit default line height 作为 scale；正式共享
  layout 接入后必须由同一个 shaped layout metrics contract 提供该换算，不能继续依赖估计值。
- adapter 只在请求仍匹配当前 Revision 时应用 absolute target，最后按 native content height
  与 clip height clamp，并通过 `reflectScrolledClipView` 提交 bounds。stale、no-op 和实际
  scroll 仍由无窗口 self-check 覆盖。
- `TextInputView` 在 Rust command result、鼠标/Accessibility selection 写回、IME commit、
  composition cancel 和 layout/resize 后执行 reveal；marked text 更新本身不推进 canonical
  selection，不用它触发 Rust caret 查询。
- AppDelegate 只负责建立 `NSScrollView` host 并连接 adapter；source、composition overlay、
  selection 和布局真源仍在现有 Rust/TextKit 实验边界内。

## 结果

- spike 现在有一条真实的 `TextInputView → NSScrollView → NSClipView` 消费路径，命令和
  commit 后可以自动滚动 caret，而不需要 GUI 产品层。
- native point 与 Rust 逻辑 viewport 单位的转换集中在 bridge，后续替换为共享布局 metrics
  时不需要改变 command 或 adapter 协议。
- 当前 Rust 默认 block/layout 仍不是 TextKit 的最终排版结果；因此本 ADR 证明的是 host、
  Revision 和消费时序，不宣称 native scroll target 已达到产品级视觉精度。

