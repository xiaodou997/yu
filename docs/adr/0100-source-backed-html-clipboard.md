# ADR 0100：Revision-bound source-backed HTML 剪贴板

## 状态

已接受（Phase 2，Rust headless + macOS host）。Rust 跨平台格式契约已固定；其他平台 native
clipboard adapter 和完整语义覆盖仍待定义。

## 背景

Yu 的 TextKit 内容只是可丢弃的输入/绘制镜像，不能直接作为剪贴板来源。此前 macOS host
只发布 Markdown UTI 和纯文本；HTML 如果直接包裹 Markdown source，或者从带 IME preedit 的
TextKit 字符串拼接，会把 Markdown delimiter、暂态 composition 和 source projection 混进错误的
富文本语义。

## 决策

- 新增 `yu-export` crate，接受一个 `TextSnapshot`、expected `Revision` 和 source `TextRange`。
  revision 或 UTF-8 boundary 不匹配时导出失败，native 回调不得读取新状态。
- `ClipboardPayload::markdown()` 是选择范围的 canonical source；`plain_text()` 当前也保留同一
  source，保证纯文本应用不会静默丢失 Markdown 语法。
- `html()` 从同一 source range 建立临时 parser fragment，输出保守 semantic HTML：heading、
  paragraph、emphasis/strong/code、link/image/autolink、reference link、fenced code、
  blockquote 和 task list 使用 parser-owned source ranges；连续且同层级的列表项共享一个
  `<ul>`/`<ol>` 容器，source depth 会生成嵌套列表，有序列表的起始编号保留；未解析的 reference
  不生成 broken `href`，而是保留可见 label。GFM table 由 `yu-markdown::parse_table` 提供
  source-relative cell ranges，导出为带 `<thead>/<tbody>`、inline cell 内容和对齐样式的 `<table>`；
  table 暂时仍是普通 paragraph block，避免在 editor/projection 尚未准备好时引入新的 block kind。
- HTML exporter 不读取 TextKit、projection 或 transient composition，也不修改 session、Revision、
  selection、history 或 dirty。
- `yu-storage-ffi` 暴露 `yu_storage_session_copy_selection_html` 的两次查询 ABI；macOS host
  copy/cut 在同一个 selection Revision 下发布 `net.daringfireball.markdown`、纯文本和
  `public.html`。
- `yu-export::ClipboardFormat` 固定三种跨平台 payload 的顺序、MIME、macOS UTI 和取值映射；
  native adapter 不应重新定义格式字符串或改变 Markdown/纯文本/HTML 的优先语义。
- paste 继续优先 Markdown UTI，再回退到纯文本；本阶段不把 HTML 作为 Yu 内部粘贴输入，避免
  HTML→Markdown 逆向解析在 parser semantic coverage 尚未完整时制造第二套真源。

## 验证

- `cargo test -p yu-export`
- `cargo test -p yu-storage-ffi`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- macOS host Swift build/人工剪贴板验收；Rust 测试至少覆盖 Unicode、stale Revision、UTF-8 boundary
  和 HTML escaping。

## 后续

继续扩展 HTML fragment 的 Markdown 语义覆盖，并定义 Windows/Linux native clipboard format
映射；HTML paste 只有在独立安全的 HTML→Markdown policy 确定后才开放。
