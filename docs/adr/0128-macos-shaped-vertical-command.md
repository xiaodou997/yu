# ADR 0128：macOS shaped vertical editor command

## 状态

已接受（Phase 3 Track B；最终 Metal hit-test 与 visual IME 仍后置）。

## 背景

Rust 编辑器已经有跨 block 的 `MoveUp/Down`、preferred-X 和 Shift 扩展语义，但原生 host
此前只能调用普通 metrics command。生产 surface 使用 CoreText line metrics 时，source
selection 可能因此落在与可见 glyph 不同的 visual line/caret 上；caret reveal 虽然使用 shaped
viewport，却无法修正 command 本身的命中。

## 决策

- `yu-editor` 将垂直移动的核心逻辑抽成可注入 layout loader，并增加
  `move_vertical_with_shaper`。source selection、preferred-X、anchor、history break 和
  `CommandResult` 语义与普通命令完全相同，shaper 只是本次 block layout 的 caller-owned 依赖。
- `yu-storage-ffi` 增加 `yu_storage_session_macos_move_vertical`。它接收 expected Revision、
  vertical command、字体 size/width，验证已发布的 CoreText viewport metrics，然后以
  `CoreTextShaper` 调用 Rust shaped layout；返回值仍是普通 `YuStorageCommandResult`，不暴露
  layout、glyph 或 AppKit 对象。
- `DocumentTextView` 在上下移动前通知 `MacosSurfaceHostCoordinator` 同步发布当前 metrics，
  随后优先走 shaped vertical FFI；若首个按键发生在 viewport 尚未发布的瞬间，才回退一次普通
  metrics command。command 完成后的 selection notification 继续触发同一 Revision 的 caret
  reveal。
- 透明 Metal surface 仍不接收键盘或鼠标，TextKit 继续作为 NSTextInputClient、IME、剪贴板和
  Accessibility owner；最终 shaped Metal point hit-test 尚未在本 ADR 中切换。

## 结果

生产窗口的 Up/Down/Shift-Up/Shift-Down 现在与当前 CoreText visual line 使用同一 width/metrics
契约，preferred-X 跨连续移动仍由 Rust 保持，selection/history/source Revision 不会被平台复制。
跨平台或未初始化 viewport 时仍保留普通 command 回退，不阻塞输入。

## 验证

```bash
cargo test -p yu-editor shaped_vertical_commands_use_caller_shaper_and_keep_revision
cargo test -p yu-storage-ffi ffi_macos_shaped_vertical_command_preserves_revision_and_selection_contract
experiments/macos-document-host/build-rust-ffi.sh
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --shaped-vertical-self-check \
  experiments/macos-document-host/Fixtures/block-projection.md
```
