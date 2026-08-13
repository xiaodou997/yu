# ADR-0079：macOS 输入源诊断与 composition Accessibility 契约

## 状态

已接受（2026-08-13）

## 背景

`NSTextInputClient` 的协议回放可以验证 `setMarkedText`、`unmarkText`、commit 和 cancel 的状态
转换，但它不能证明用户当前选择的 macOS 输入源真的产生了同样的事件。相同地，直接调用
`NSAccessibility` 方法只能验证 AX text-entry 契约，不能代替 VoiceOver 的实际朗读验收。

## 决策

macOS spike 启动时在主线程通过 HIToolbox `TextInputSources` API 记录当前选中的键盘输入源：

```text
identifier
localized name
input-source type
```

该探针只读、不切换输入源，也不把输入源信息写入 Yu 文档状态。它用于让人工测试日志明确记录
“测试时到底使用了哪个输入源”。

同时增加 composition-aware AX self-check，验证 marked preedit 和 unmark presentation transition
期间以下值保持同一份 native mirror：

```text
AX value / character count
AX selected text range
AX marked range text and underline attribute
AX marked range geometry
commit exactly once
cancel restores canonical mirror
```

## 后果

- 日文输入源、dead key、组合重音的真实按键仍须在 macOS 上人工切换输入源后验收；自动回放不再被
  误报为真实输入源测试。
- VoiceOver 实际朗读质量仍须人工开启 VoiceOver 验证；AX API contract self-check 只保证
  VoiceOver 可以读取到稳定的 text-entry 数据。
- HIToolbox 调用必须留在 macOS 平台 bridge，并且只在主线程执行。
