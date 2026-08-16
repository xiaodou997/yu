# ADR 0124：macOS product NSView surface lifecycle

## 状态

已接受（Phase 3 Track C；surface adapter 已进入 document-host 窗口生命周期，TextKit/source
mirror 仍是用户可见编辑表面）。

## 背景

ADR 0123 已证明 Rust storage session 可以持久拥有同一 `NSView` 的 `CAMetalLayer`、renderer、
atlas 和 generation，但验证仍局限在临时 self-check window。产品窗口需要把窗口 attach、layout、
resize、scroll、Revision 更新和 close 映射到同一个 FFI 协议，同时不能因为 surface adapter 的
存在而引入第二套 Markdown 或 selection 模型。

## 决策

- document-host 增加 `MacosSurfaceHostView`。它只报告 `viewWillMove(toWindow:)`、
  `viewDidMoveToWindow()` 和 `layout()`，不解析文本、不绘制 source、不拥有 Rust state。
- `MacosSurfaceHostCoordinator` 由 `DocumentViewController` 持有，使用 `NSView` bounds、
  `NSScrollView` content bounds、backing scale 和 Rust `Revision` 生成 submit key；同一 key 不
  重复提交，scroll、resize、编辑和字体变化会安排下一次主线程提交。
- 新增 `yu_storage_session_macos_font_metrics`，让 coordinator 在空 Markdown（没有 parser block）
  时也能取得 CoreText line height/default advance 并配置 viewport；metrics 与 submit 都必须经过
  当前 Revision gate。
- surface host view 是 source TextKit mirror 的 sibling，并置于其后方。它可以真实 attach 和
  submit drawable，但不替换用户当前看到的 TextKit source mirror，也不暴露 native layer/GPU
  handle 给 Swift 或 shared editor model。
- view 离开 window、窗口允许关闭或 controller 销毁时都调用幂等 detach；所有 surface FFI 调用
  仍只在 AppKit main thread 执行。

## 结果

正常 document-host 窗口现在拥有真实 surface lifecycle adapter，覆盖 attach、resize、scroll、编辑
Revision 和 close detach；`--macos-render-host-lifecycle-self-check` 对同一流程建立了可重复的
AppKit/CAMetalLayer 回归。视觉编辑仍使用 source TextKit mirror，heading/emphasis/code/link 的
正式 Rust visual renderer 尚未切换。

## 验证

```bash
cargo test -p yu-storage-ffi -p yu-render-macos
cargo clippy --workspace --all-targets -- -D warnings
./experiments/macos-document-host/build-rust-ffi.sh
swift build --package-path experiments/macos-document-host
experiments/macos-document-host/.build/debug/YuMacDocumentHost \
  --macos-render-host-lifecycle-self-check \
  experiments/macos-document-host/Fixtures/block-projection.md
```
