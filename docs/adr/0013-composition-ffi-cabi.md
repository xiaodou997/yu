# ADR 0013：CompositionOverlay 使用窄 C ABI 连接原生平台

- 状态：Accepted
- 日期：2026-08-10

## 背景

macOS 的 `NSTextInputClient` 使用 Swift/Objective-C 对象、`NSRange` 和 UTF-16 坐标，而
`yu-editor` 的 composition 协议使用 Rust `TextRange`、`Utf16Range` 与 `TextBuffer`。平台层
需要一个可以在 Swift、Objective-C、C 或其他原生 shell 中调用的稳定边界，但不能把 Rust 的
存储、生命周期或泛型类型暴露出去。

## 决策

新增独立的 `yu-editor-ffi` crate，构建为 Rust `staticlib`，仅暴露一个 opaque
`YuCompositionSession` 句柄和状态码。C ABI 的输入输出约束如下：

```text
UTF-8 text  ── pointer + byte length ──► Rust
NSRange     ── UTF-16 start/end       ──► Rust conversion
Rust state  ── opaque session          ──► platform owns handle only
```

session 内部拥有 `TextBuffer` 和可选的 `CompositionOverlay`，公开操作只有：

- `new/reset_source`：创建或替换 canonical source；有 active overlay 时拒绝 reset；
- `begin/update`：创建或更新 preedit overlay，替换范围从当前 source UTF-16 映射到 UTF-8
  byte range；
- `commit`：把 overlay 转为一个 Transaction 并应用到 Rust buffer；成功后清除 overlay；
- `cancel`：丢弃 overlay，不修改 source 或 Revision；
- `revision/source/copy` 查询：通过 caller-owned buffer 复制结果，不返回 Rust-owned pointer。

除只释放 opaque handle 的 `destroy` 外，所有函数都返回显式 `int32_t` status。非法 UTF-8、
空句柄、UTF-16 越界、surrogate/scalar 中间位置、没有 overlay 或输出 buffer 太小，都必须
变成可检查的错误码。

## 结果

- Swift `NSTextInputClient` 可以保持原生 `NSRange`，只在 bridge 处转换成 Rust 协议；
- preedit 仍然不会写入 canonical `TextBuffer` 或 Undo；
- Rust 静态库不依赖 Swift runtime，其他平台可以复用同一窄 ABI；
- opaque handle 限制了 ABI 的长期承诺，后续可以替换内部 Piece Tree、Snapshot 或 overlay
  实现，而不改平台调用方；
- 当前 `copy_source` 是用于 spike 和 AX 自检的显式复制接口，正式大文档路径应改成带
  Revision 的局部查询，避免物化整个 source。

## 非目标

这个 ABI 不是最终插件 ABI，也不传输异步 callback、线程安全承诺、UI 对象或 GPU 资源。第一版
session 由单个 `NSTextInputClient` 在主线程拥有；并发/跨线程访问要等正式 editor state 协议
确定后再设计。
