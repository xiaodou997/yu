# ADR 0107：macOS native selection 的双入口同步

## 状态

已接受（Phase 2，macOS host）。AppKit 鼠标/拖选与键盘命令现在都回到同一个 Rust selection；
原生多选区 UI 仍不作为 Yu editor model 的承诺。

## 背景

`DocumentTextView` 之前只重写 `setSelectedRange`。键盘命令会通过 Rust command 更新 selection，
但 AppKit 的鼠标点击、拖选和部分 TextKit Accessibility 路径使用 `setSelectedRanges`，导致
TextKit mirror 看似移动而 Rust 仍保留旧位置。下一次 paste/edit 重新投影时，旧 Rust selection
又把光标恢复到固定行。

## 决策

- 同时覆盖 `setSelectedRange` 和 `setSelectedRanges(_:affinity:stillSelecting:)`；两条入口都先
  让 AppKit 更新 disposable mirror，再把最终 native range 通过 Revision-bound
  `yu_storage_session_set_selection` 写回 Rust。
- Yu 当前 editor model 只有一个 selection。`setSelectedRanges` 只取 AppKit 最终的单一
  `selectedRange()`，不把多个 `NSValue` ranges 复制进 Rust，也不在 Swift 维护第二个 selection。
- `synchronizingSelection` 继续阻止 `synchronizeProjection()` 的反向镜像更新再次触发 native→Rust
  回调；composition active 时 native selection 仍由 composition overlay 管理。
- selection 同步失败只报告 bridge error，不静默把 TextKit 的旧位置当成 canonical source state。
- `--selection-self-check` 在无窗口模式下依次设置两个不同 source position，验证两次都到达 Rust；
  clipboard 与 Accessibility self-check 作为回归一起运行。

## 验证

```bash
experiments/macos-document-host/build-app.sh
experiments/macos-document-host/.build/YuMacDocumentHost.app/Contents/MacOS/YuMacDocumentHost \
  --selection-self-check experiments/macos-document-host/Fixtures/sample.md
```

## 后续

真实窗口中继续验证鼠标点击、拖选后粘贴/输入、键盘上下移动和 IME composition 的交错顺序；
完整多选区编辑模型、跨平台 selection adapter 留到 native editor shell 阶段。
