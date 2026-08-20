# macOS Accessibility Spike：2026-08-09

## 环境

- macOS / AppKit 自定义 `NSView`
- `NSAccessibility` text area 协议
- `AXUIElement` 参数化属性运行时探针
- TextKit 仅用于本次 shaping 与几何验证

## 实现范围

实验 View 暴露 `AXTextArea` role、value、UTF-16 字符数、selection、visible range、逻辑行、
range 文本、point/range 映射和 screen-space bounds。文本或 selection 变化后分别发送
`.valueChanged` 与 `.selectedTextChanged` 通知。

Rust `yu-editor` 同时增加 `AccessibilityTextSnapshot`，验证 source byte range 与 AppKit UTF-16
range 的双向转换、Revision 一致性和 Piece Tree chunk 查询。

## 实测结果

应用启动后的内部一致性检查输出：

```text
AX self-check characters=47 selection={47, 0} firstLine={0, 19}
```

随后使用系统 `AXUIElement` 从进程的 focused element 查询，结果为：

```text
trusted=true
role=AXTextArea
characters=47
string="Yu macOS IME spike\n"
bounds={x:1103, y:374, w:712, h:26}
```

role、字符数、`AXStringForRange` 和 `AXBoundsForRange` 均返回成功状态。由此确认自绘 View 可以
在不继承 `NSTextView` 的前提下提供系统可消费的文本范围与屏幕坐标。

## 结论与剩余风险

- AppKit/AX 文本位置使用 UTF-16，Rust source 仍使用 UTF-8 byte offset；转换必须绑定 Revision。
- `AXBoundsForRange` 必须使用与文本查询一致的 Layout，不应由 core 估算。
- 现有 spike 的 TextKit storage 已包含 marked text；正式实现需要让 Rust `CompositionOverlay`、
  Accessibility query 与 caret geometry 共享一次原子发布的编辑状态。
- 本次验证覆盖系统 AX API 查询，没有覆盖 VoiceOver 的实际朗读质量和大型虚拟化文档的
  accessibility tree 策略。

## AX tree / VoiceOver 状态复核

通过 macOS Computer Use 读取运行中窗口的 AX tree，Yu View 显示为：

```text
text entry area Description: Yu Editor document
ID: yu-editor-document
Value: Yu macOS IME spike
```

系统设置中的 VoiceOver 开关当前为 `off`。本次没有修改该系统设置，因此实际 VoiceOver 朗读
仍保持待人工验证；AX element、role、value 和参数化 range 查询已分别通过。
