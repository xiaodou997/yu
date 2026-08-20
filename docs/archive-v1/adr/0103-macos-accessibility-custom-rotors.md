# ADR 0103：macOS Accessibility Heading/Link custom rotors

## 状态

已接受（Phase 2，macOS host）。Rotor 的真实 VoiceOver 操作仍需人工验收；本 ADR 只固定 native
查询契约，不承诺跨平台 API。

## 背景

语义 child tree 让 VoiceOver 能看到标题、列表项和链接，但逐个遍历所有段落/inline child 不是文档
导航的可用交互。macOS 的 custom rotor 可以让 VoiceOver 按标题或链接快速跳转，同时保留 Rust
source 作为唯一真源。

## 决策

- `DocumentTextView` 暴露两个 custom rotor：系统 Heading rotor 和系统 Link rotor。
- rotor delegate 每次查询都从当前 Revision 的 owned child tree 展开候选，不缓存字符串或独立索引。
- Heading rotor 只返回 `heading` semantic kind；Link rotor 返回 `link`、`autolink` 和
  `referenceLink`。
- `filterString` 使用当前 child 的 source-backed `accessibilityLabel` 做大小写不敏感过滤；查询方向
  不循环，越过文档边界返回 nil。
- `ItemResult.targetElement` 指向实现 `NSAccessibilityElementProtocol` 的 semantic child，
  `targetRange` 使用同一节点的 source UTF-16 range。这样 VoiceOver 聚焦与 source range 查询仍然
  绑定同一个 Revision。
- delegate 由 `DocumentTextView` 强引用，因为 AppKit 的 rotor delegate 属性是 weak；source refresh
  先销毁旧 semantic children，再建立新树，避免 rotor 保留 stale element。

## 验证

```bash
CLANG_MODULE_CACHE_PATH="$PWD/.cache/clang" \
SWIFTPM_MODULECACHE_OVERRIDE="$PWD/.cache/swiftpm" \
swift build --package-path experiments/macos-document-host

experiments/macos-document-host/.build/arm64-apple-macosx/debug/YuMacDocumentHost \
  --accessibility-self-check experiments/macos-document-host/Fixtures/sample.md
```

无窗口 self-check 验证两个 rotor 都能返回当前 Revision 的标题/链接 child，并同时检查 task checkbox
的布尔状态与编辑后的 stale node。VoiceOver 打开后的 rotor 手势、朗读语言和焦点视觉反馈必须在
真实 macOS 会话中记录。

## 后续

URL/图片 action、表格 cell child role、焦点恢复策略和跨平台 rotor abstraction 暂不进入本阶段；
URL 属性与 task checkbox press 的 source/action 边界见 ADR-0104。其它动作必须先定义 source
Transaction 与 Revision 语义再实现。
