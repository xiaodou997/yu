# ADR 0001：Markdown source 是持久化真源

- 状态：Accepted
- 日期：2026-08-09

## 决策

Yu 不建立可独立序列化的 Rich Text document model。Markdown 字节序列是唯一持久化真源，
语法、投影、布局、场景和可访问性均为派生表示。

## 结果

- 保存不需要从视觉结构反向生成 Markdown；
- 用户原有 delimiter、空白和书写风格可以保留；
- 所有派生节点必须具备 Source Range 或明确的 synthetic 身份；
- malformed source 必须仍可编辑；
- Export HTML/PDF 是独立边缘管线，不成为编辑管线的一部分。

