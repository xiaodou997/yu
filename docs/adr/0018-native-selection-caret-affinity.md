# ADR 0018：原生 selection FFI 保留 CaretAffinity

- 状态：Accepted
- 日期：2026-08-10

## 背景

ADR 0017 已经让 AppKit 的 UTF-16 selection 写回 Rust，但当鼠标命中软换行或硬换行边界时，
同一个 UTF-16 offset 可能有 upstream/downstream 两个视觉 caret。若 FFI 始终构造
`CaretAffinity::Downstream`，AppKit 的视觉位置与 `EditorDocument` 会再次分叉。

## 决策

selection ABI 使用两个稳定的 `uint8_t` affinity 值：

```c
YU_CARET_AFFINITY_UPSTREAM = 0
YU_CARET_AFFINITY_DOWNSTREAM = 1
```

`yu_composition_session_set_selection` 接收 affinity，`yu_composition_session_selection` 同时
返回 affinity。Rust 只接受这两个值，并把它转换为 `CaretAffinity`；未知值、stale Revision
和非法 UTF-16 range 都拒绝且不修改 selection。

macOS bridge 的映射规则是：

```text
NSSelectionAffinity.upstream   ⇄ YU_CARET_AFFINITY_UPSTREAM
NSSelectionAffinity.downstream ⇄ YU_CARET_AFFINITY_DOWNSTREAM
```

mouse hit-test 传递它计算出的 affinity；Accessibility selection 和普通键盘移动使用
downstream；Rust 查询返回的 affinity 供 bridge 在 commit/self-check 后校验。

## 结果

- 软换行/硬换行处的 native caret 不再被 FFI 静默归一为 downstream；
- selection 的 range、Revision 和视觉 affinity 作为一个状态一起跨越 ABI；
- C ABI 仍只传递标量和 caller-owned 输出，不暴露 Rust 类型；
- 现阶段仍是 identity projection，Markdown delimiter 隐藏后的 affinity 映射留给
  `yu-projection`/`yu-layout`。

## 限制

`uint8_t` 值是当前内部 ABI 约定，不是最终插件 ABI；正式跨平台 adapter 需要为 Windows/GTK
定义同等的 affinity 映射和错误处理。
