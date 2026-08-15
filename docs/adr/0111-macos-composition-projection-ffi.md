# ADR 0111：macOS composition projection FFI

## 状态

已接受（Phase 3 Track A）。

## 背景

`DocumentEditorSession` 已经把 AppKit `NSTextInputClient` 的 marked text 作为 Rust-owned
`CompositionOverlay` 管理，但 storage FFI 之前只能返回 preedit 原文。native host 若自行把
preedit 拼到 Markdown projection，会重复实现 hidden syntax、UTF-8/UTF-16 和 selection 映射，
并可能在同一 source Revision 下接受过期的 update。

## 决策

storage FFI 复用 `Projection::with_composition`，新增三条 revision/generation-aware 查询：

1. `yu_storage_session_composition_projection` 返回 replacement range、preedit selection、
   visual selection、projected UTF-8/UTF-16 length 和当前 generation；
2. `yu_storage_session_copy_composition_projection` 以 count/fill 返回 owned projected UTF-8，
   同时校验 expected Revision + generation；
3. `yu_storage_session_composition_caret` 先验证 canonical source UTF-16 boundary，再返回
   preedit selection active end 对应的 visual UTF-16 caret、visual selection 和 round-trip source。

metadata 查询失败前先清空 output；generation 过期、无 overlay、surrogate split 或 projection
错误不得写入半成品结果。Swift 只保存 owned scalars/bytes 和两个版本号，不构造第二套
Markdown projection。

## 后果

- native composition mirror 可以原子丢弃旧 generation，并保持 source/Revision 不变；
- Unicode preedit、emoji、hidden emphasis/link delimiter 与 visual selection 坐标统一由 Rust
  计算；
- 当前接口仍是诊断/桥接边界，生产 TextKit mirror 尚未切换到 projected selection；point hit-test、
  block-local composition layout 与最终 retained scene 留待后续阶段。
