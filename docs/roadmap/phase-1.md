# Phase 1：Contracts & Risk Spikes

## 目标

第一阶段的目标不是完成编辑器，而是证明 Yu 最关键的数据契约和 macOS 输入路径能够成立，
并为后续 Piece Tree、增量 Markdown 与 Projection 提供不会频繁变化的边界。

## Track A：Core contracts

- [x] Rust Workspace 与固定工具链
- [x] `ByteOffset`、`TextRange`、`Revision`、`TextAnchor`、`Affinity`
- [x] 不可变 `TextSnapshot`
- [x] 原子多 edit Transaction
- [x] ChangeSet、Anchor 映射和 inverse Transaction
- [x] 参考 UTF-8 文本后端
- [x] lossless Markdown block scanner
- [x] `yu-inspect` CLI
- [x] 可重复运行的 parse/edit 参考 benchmark harness
- [x] 持久化 Piece Tree 与 Persistent Rope 初代候选及共同 workload benchmark
- [x] 确定性随机 Transaction model test（2,000 次 Unicode edit/inverse）
- [x] Piece/leaf 局部合并与 insert/inverse 结构稳定性测试
- [x] 多版本 Snapshot 共享分配测量并选择 Piece Tree 主后端
- [x] Piece Tree 行数与 UTF-16 长度摘要、chunk cursor
- [x] Chunk-aware 完整解析与保守增量解析 differential harness
- [x] 带 start/end state、hash 与 suffix reuse 的持久化 block sequence
- [x] 长期增量 session、block retention 统计与 idle compaction 策略

## Track B：macOS risk spike

- [x] 可编译的 AppKit 实验程序
- [x] `NSTextInputClient` 的 marked text/commit/candidate rect 最小链路
- [x] 人工验证中文拼音、emoji 与 Escape cancel
- [ ] 人工验证日文、dead key 与组合重音
- [x] 日文、组合重音与 cancel 的 NSTextInputClient 协议回放
- [x] 将实验事件转换为 Rust `CompositionOverlay` 协议
- [x] 通过 C ABI static library 完成 Swift ↔ Rust `CompositionOverlay` smoke test
- [x] `EditorDocument` 统一拥有 canonical source、Revision 与 composition overlay
- [x] FFI revision-bound 局部 UTF-8 source query（不物化完整 Snapshot）
- [x] `EditorSelection`、caret affinity 与基础 Unicode command 模型
- [x] FFI selection revision/UTF-16 查询并接入 macOS composition commit 自检
- [x] 系统 Accessibility text range 与 screen bounds 查询实验
- [x] Yu View AX text entry tree 运行时查询
- [ ] VoiceOver 实际朗读质量验证
- [x] 多行 shaping、点击和 caret round-trip

## Phase 1 退出条件

进入完整编辑器垂直切片前，必须满足：

1. 随机编辑下 Transaction + inverse 保持内容正确。
2. 新文本存储后端通过同一套行为测试。
3. Markdown 增量结果与完整解析结果可自动比较。
4. macOS 拼音 composition 不把 preedit 写入 Undo。
5. `SourceCaret → NativeCaret → Point → NativeCaret → SourceCaret` 有 identity projection
   下的最小可验证闭环。
6. 形成第一份真实性能基线，而不是仅有目标数字。
7. selection、composition commit 和永久 Transaction 使用同一个结果 Revision。

## 非目标

- 完整 CommonMark/GFM；
- 产品级窗口、菜单或设置页；
- 自研字体 shaping；
- 三个平台同时达到产品质量；
- 第三方插件 ABI。
