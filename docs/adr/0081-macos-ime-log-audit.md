# ADR-0081：macOS IME 结构化日志审计

## 状态

已接受（2026-08-13）

## 决策

macOS spike 提供无窗口命令：

```text
YuMacTextInputSpike --audit-ime-log PATH
```

它读取包含 `IME_EVENT {json}` 的终端日志并验证：

```text
sequence 连续
composition replacement start 稳定
generation 单调递增
preedit/unmark/commit/cancel 使用正确 native range
composition 期间 canonical Revision 不变
```

日志末尾因 Ctrl-C 造成的不完整 JSON 允许作为 `truncatedTail=true`；任何中间损坏事件仍然失败。
审计器不重放 GUI、不修改 Rust document，也不宣称真实输入源或 VoiceOver 已完成验收。

## 后果

- 日文输入源、dead key、组合重音的人工事件可以保存后重复检查，而无需再次打开窗口。
- 审计器检查的是事件协议和坐标状态，不验证候选词 UI 的视觉质量。
- fixture 只覆盖最小协议序列，真实输入日志仍应保留输入源 probe 和交互上下文。
