# ADR 0101：source-backed Markdown Accessibility 语义树快照

## 状态

已接受（Phase 2，Rust headless + macOS FFI）。AppKit child element 和跨平台 Accessibility
适配仍待后续阶段。

## 背景

`AccessibilityTextSnapshot` 已能提供 Revision-bound 的 UTF-16 文本、选区和逻辑行查询，但
VoiceOver 还需要知道标题、段落、代码块、列表项以及链接等 Markdown 语义。如果让 Swift 从
TextKit 镜像重新解析 Markdown，就会重新产生第二套 parser、source range 和 Revision 语义。

## 决策

- `yu-editor::AccessibilitySemanticSnapshot` 从当前 `EditorDocument` 的 canonical source 和
  parser-owned block/inline spans 构建节点序列；节点 0 永远是 document root，block 节点挂在 root
  下，inline semantic span 挂在所属 block 下。
- 每个节点只拥有稳定 kind、parent/index、level/flags 和 source/label UTF-16 ranges；文本仍由
  已有的 Revision-bound source range query 按需读取。快照创建不会改变 selection、Revision、
  history 或 source。
- heading label 去掉 ATX marker；其它当前支持的 block 默认使用完整 source range，后续 projection
  或 layout 可以在不改变 ABI 的情况下细化 label policy。
- `yu-storage-ffi` 提供 count/fill 两个查询。调用方必须传入 expected Revision；stale Revision、
  无效 parser range 和输出容量错误都返回已有状态码，不能返回跨 Revision 的半旧树。
- Swift host 只把 C struct 转成 owned scalar 节点；本阶段不实现完整 `NSAccessibilityElement`
  child tree，也不把节点文本缓存为第二份 Markdown。

## 验证

- Rust editor test 覆盖 document root、heading label、strong/link、task done flag 和编辑后新
  Revision。
- FFI test 覆盖 count/fill、parent/kind/flag、UTF-16 label range 和 stale Revision。
- macOS document host 通过 header import 与 Swift owned-node bridge 编译验证；VoiceOver 实际
  朗读仍必须由人工验收记录。

## 后续

下一阶段将把该节点序列映射为 AppKit VoiceOver child elements，并明确 geometry、actions、hit-test
和跨平台 role mapping；在此之前不得让 Swift 或 TextKit 自己推导 Markdown 语义。
