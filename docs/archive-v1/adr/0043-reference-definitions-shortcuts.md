# ADR 0043：Markdown reference definitions and shortcut references

## 状态

已接受（Phase 1）

## 背景

0042 已经给显式/collapsed reference link 建立了 lossless inline span，但 `[label]` 形式是否
是普通文字不能由 inline parser 单独决定：它依赖文档中其他位置的 `[label]: destination`
定义。若让 projection 自己扫描定义，parser、projection 和 cache 会各自拥有一套不一致的
语义。

## 决策

- 根级 block parser 将保守的 `[label]: destination` 行记录为 `BlockKind::ReferenceDefinition`。
  只允许最多三个 ASCII 空格缩进，destination 可以是 `<...>` 或不含空白的 token；title 暂时
  不建模，整行仍保持 source-backed。
- `MarkdownDocument` 为每个 source revision 持有 `ReferenceDefinitionIndex`。索引只保存
  label/destination source range 和规范化 hash，不复制正文；lookup 只接受同一 revision，并做
  ASCII case-fold 与空白折叠。
- `parse_inline_with_definitions` 在 index 命中时识别 `[label]` 与 `![label]` shortcut
  reference；显式 `[label][id]`、collapsed `[label][]` 和 inline `[label](url)` 优先保持原有
  语义。没有 index 或没有命中时，shortcut 保持普通可编辑源码。
- definition block 使用零宽 `BlockProjection::ReferenceDefinition`；普通 block 的 projection
  消费同一 index，绝不从源码重新猜测 definition。
- definition fingerprint 描述定义顺序、label 与 destination 内容，不包含绝对 offset。前缀编辑
  可以继续 remap projection；新增、删除或修改 definition 时，`EditorDocument` 清空
  projection/layout/viewport cache，因为影响可能传播到文档远处的 shortcut reference。

## 结果

- Markdown source 仍是唯一真源；定义、shortcut label 和 destination 都能反向映射到源码。
- Typora 风格的 shortcut label 可以隐藏 `[]`，但只有存在明确文档定义时才隐藏，不会把普通方括号
  文本误认成链接。
- definition 语义变化的失效边界是可测试且保守的；缓存不会继续使用旧的非局部解析结果。

## 限制

本阶段不实现完整 CommonMark definition grammar、嵌套/转义 label、title AST、URI 规范化、
定义作用域、footnote 或 HTML block；这些语义必须在扩展 parser 中明确加入，不能由 projection
猜测。definition 行当前只在根级 block 识别，容器内的相似文本仍属于容器内容。
