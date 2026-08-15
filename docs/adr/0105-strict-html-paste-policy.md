# ADR 0105：受控 HTML→Markdown 粘贴策略

## 状态

已接受（Phase 2，Rust headless + macOS host）。策略已经实现并有单元测试；macOS adapter 已经
消费这个策略，Windows/Linux native adapter 仍待定义。

## 背景

Yu 的 Markdown source 是唯一真源。剪贴板 HTML 只是由 source range 派生的互操作格式，不能被
浏览器式 HTML parser、DOM 或 TextKit mirror 反向解释成第二份富文本模型。另一方面，其他应用
可能只提供 `public.html`/`text/html`，如果永远无条件降级纯文本，会丢掉标题、链接、列表和表格等
明确的语义。

因此需要一个可以独立测试、可拒绝、不会执行 HTML 的 HTML→Markdown 边界。策略的目标不是兼容
任意网页，而是接受 Yu 自己生成的 semantic HTML 以及等价的安全片段。

## 决策

- `yu_export::import_html_fragment` 只接受显式 allowlist：段落、标题、强调、代码、链接、图片、
  换行、引用、列表/task checkbox、fenced code 和带 `thead`/`tbody` 的 GFM table。
- parser 是受限的 fragment parser，不是浏览器 HTML parser；未知标签、注释、未允许属性、未闭合/错配
  标签、非法 entity 和不符合结构的节点都返回 `HtmlImportError`，调用方必须回退到 `text/plain`。
- URL 只保留无控制字符且不以 `javascript:`、`vbscript:` 或 `data:` 开头的 destination；策略不会
  打开 URL、加载图片、执行脚本、解析 CSS 或创建 native object。
- 标签属性也采用 allowlist：链接只允许 `href`，图片只允许 `src`/`alt`，代码 class 只允许
  `language-*`，ordered list 只允许正整数 `start`，table cell style 只允许三个 `text-align`
  值；checkbox 只接受 disabled checkbox input。
- 导入结果始终是 Markdown source 字符串。文本中的 Markdown punctuation 会被转义，代码块会根据
  内容选择足够长的 backtick fence，表格对齐从 cell style 映射到 GFM delimiter。
- 该模块不改变 `TextBuffer`、Revision、selection、history 或 session。native adapter 先尝试
  Markdown payload，再尝试纯文本，最后才调用本策略；这样已有 lossless source 不会被 HTML 重新
 解释。没有纯文本时，任何 `HtmlImportError` 都是可观测的拒绝并放弃粘贴，不能猜测或拼接另一套
  rich-text model。

## 验证

```bash
cargo test -p yu-export
cargo clippy -p yu-export --all-targets -- -D warnings
```

测试覆盖 Yu semantic fragment、Unicode/entities、task checkbox、代码 fence、GFM table 对齐、
export→import round trip，以及未知标签、属性、危险 URL、错配结构和非法 entity 的拒绝路径。

## 后续

下一步定义 Windows/Linux native clipboard adapter 的格式优先级、错误/回退遥测和平台 API 映射；
macOS 的 HTML fallback 已通过独立 C ABI、私有 pasteboard self-check 接入，但真实跨应用 paste
仍需人工回归。
