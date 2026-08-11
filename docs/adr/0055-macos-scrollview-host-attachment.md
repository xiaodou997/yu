# ADR 0055：macOS Spike 真实 `NSScrollView` Host Attachment

## 状态

已接受（Phase 1）

## 背景

ADR 0054 只验证了一个无窗口 `NSScrollView` 能消费 Rust 的 absolute caret scroll
request。下一步需要证明同一个边界可以承载真实 `TextInputView`，否则命令、IME commit 和
Accessibility selection 改变后，caret reveal 仍然只是孤立的协议测试。

macOS spike 的 TextKit 使用 native point 作为绘制和 `NSClipView` 坐标，而 Rust viewport
实验默认使用 line-height 为 1、grapheme advance 为 1 的逻辑单位。让任一侧直接假设另一侧
的单位相同，会把当前风险实验的尺寸差异错误地固化成产品契约。

## 决策

- `TextInputView` 作为 `NSScrollView.documentView`，`YuNativeViewportAdapter` 仍只拥有
  `NSScrollView` 的 viewport 状态；Rust 不接触 AppKit 对象。
- adapter 在每次同步时维护当前 Revision 和 TextKit `usedRect` 推导的 native content
  height，并从 `NSClipView.bounds` 读取当前 native `scroll_y` 与 viewport height。
- bridge 通过 revision-bound `yu_composition_session_set_viewport_config` 发布
  `max_width`、`line_height`、metrics-only `default_advance`、`estimated_block_height` 和
  `overscan`。Rust 随后直接在 native point 单位计算 caret geometry 和 absolute target，不再
  在 Swift 中对请求乘除临时 scale。该 FFI 配置不推进 source/selection Revision，且拒绝 stale
  Revision 或非法 metrics。
- `TextInputView` 从当前 TextKit container 和 font 计算这些值；`default_advance` 是尚未接入
  native shaper 时的保守 fallback，不代表最终字体 shaping。共享 shaped layout 接入后应由
  `GlyphRun`/`TextShaper` 提供真实 advance，并继续使用同一 viewport coordinate contract。
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
- Rust 与 native host 现在共享显式的 viewport metrics，后续替换为 shaped metrics 时不需要
  改变 command、caret request 或 adapter 协议。
- 当前 Rust 默认 block/layout 仍不是 TextKit 的最终排版结果；因此本 ADR 证明的是 host、
  Revision 和消费时序；metrics-only fallback 仍不宣称 native scroll target 已达到产品级
  字体排版精度。
