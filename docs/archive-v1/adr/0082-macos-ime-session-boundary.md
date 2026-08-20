# ADR-0082：macOS IME 事件以 session 边界和场景标签归档

## 状态

已接受（2026-08-13）

## 背景

`NSTextInputClient` 的启动自检和人工输入事件都写入同一份终端日志。只有事件序列时，无法可靠
区分一次人工验收的起点、输入法快照和场景（例如日文罗马字、dead key 或组合重音）；把多次运行
拼接在一起也容易掩盖 session 边界错误。

## 决策

交互捕获开始时先输出一条 `IME_SESSION` JSON 记录，包含：

- 随机 `sessionID`；
- `YU_IME_SCENARIO` 场景标签；
- 当前输入源 identifier/name/type 快照；
- 第一条事件的 sequence 起点。

之后每条 `IME_EVENT` 都复制 `sessionID` 和 `scenario`。无窗口审计器默认兼容没有 session 元数据的旧
日志，以便审计历史实测；`--strict` 用于 fixture/CI，要求 session 元数据一致、没有被 Ctrl-C 截断的
尾部，并且 composition 在文件结束前已经 commit 或 cancel。

## 影响

这不会改变 Rust canonical source、CompositionOverlay 或 Undo history；session/scenario 只属于
诊断证据。它能把“协议审计通过”和“真实输入法人工验收通过”清楚分开，也为后续把日志接入可重复
回放工具保留稳定关联键。VoiceOver、真实日文输入源和 dead key 的结论仍不能由日志格式自动推断。
