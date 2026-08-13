# ADR-0084：macOS 真实输入验收使用场景契约

## 状态

已接受（2026-08-13）

## 背景

协议回放已经覆盖日文 preedit、组合重音、commit/cancel 和 Accessibility 几何，但它不能证明用户
当前真的选择了日文输入源、dead key 布局或 VoiceOver，也不能防止把某个场景的日志误归档为另一个场景。
手工测试如果只依赖终端口头说明，后续很难复现输入源和日志边界。

## 决策

1. 真实输入测试使用符合小写字母/数字/连字符格式的 `YU_IME_SCENARIO` 标签，例如 `japanese-romaji`、`japanese-kana`、
   `dead-key`、`combining-mark` 和 `voiceover`。
2. spike 提供捕获脚本和严格审计脚本；捕获脚本只设置标签并保存原始 stdout/stderr，不自动切换输入源、
   不修改 VoiceOver、不把输入写入仓库。
3. `--expect-scenario NAME` 是可选的无窗口审计约束。指定后必须存在 `IME_SESSION`，且 session 的
   scenario 必须精确匹配 NAME；旧日志仍可在未指定该选项时按既有规则审计。
4. 日文候选、dead key 组合和 VoiceOver 朗读的“是否正确”仍由人工记录模板判定，不能由协议审计器伪造
   为自动通过。

## 影响

输入验收有了稳定的场景边界和原始证据复用路径；增加的 CLI 选项向后兼容，脚本不会改变 macOS 全局输入
设置。真实人工验收仍是 Phase 1 的明确未完成项，直到用户在目标机器上填写并保存结果。
