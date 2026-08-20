可以，而且我反而建议你**不要只研究 Rust Markdown 编辑器**。真正值得借鉴的，往往是 CodeMirror 6、ProseMirror、Lexical 这些成熟编辑器框架的“编辑模型”，再用 Rust 实现自己的版本。

你现在如果继续“从输入框 → Markdown 解析 → 渲染”这样一点点往上堆，很容易越写越复杂。更适合你的方向是：**先把编辑器内核抽象出来，再做 Markdown。**

我刚重新查了一遍目前这些项目的状态。下面这几个我认为最值得研究。

---

# 一、第一梯队：建议你重点读的 6 个项目

## 1. Helix —— 最值得参考的 Rust 编辑器内核

**优先级：★★★★★**

Helix 是我最建议你研究的 Rust 项目之一。

它的架构文档明确说明，核心设计大量参考了 CodeMirror 6：

> Buffer → Rope → Selection → Transaction → Syntax

Helix 使用 Rope 作为文本底层，并且强调很多核心 primitive 是偏函数式的：操作产生新的状态，而不是到处直接修改共享状态。([GitHub][1])

对你的 Markdown 编辑器最值得借鉴的是：

```text
Document
 ├── Rope
 ├── Selection
 ├── Syntax
 ├── History
 └── Diagnostics

       ↓

Transaction

       ↓

New Document State
```

尤其不要写成：

```rust
editor.insert_text()
editor.delete_text()
editor.move_cursor()
editor.update_markdown()
editor.update_render()
```

更推荐：

```rust
Transaction {
    changes,
    selection,
    annotations,
}
```

然后统一：

```rust
state.apply(transaction)
```

这会让后面的：

* Undo / Redo
* 多光标
* 插件
* Markdown 自动格式化
* AI 修改
* 协同编辑

容易非常多。

**建议重点看：**

* `helix-core`
* `helix-view`
* `Selection`
* `Transaction`
* `Rope`
* Tree-sitter integration

---

# 二、Zed —— 学“现代 Rust GUI 编辑器应该长什么样”

**优先级：★★★★★**

如果 Helix 是编辑器内核教材，那么 Zed 更像是：

> 2026 年 Rust GUI 编辑器架构参考答案之一。

Zed 本身是 Rust 写的高性能编辑器。([GitHub][2])

特别值得研究的是 Zed 的 **SumTree**。

Zed 最近对自己的底层架构有一个很好的总结：

> 一个基于 B+ Tree 的 SumTree 数据结构同时支撑 Rope、CRDT 和 syntax map，并能够对 byte offset、line number、UTF-16 offset 做 O(log n) 查询。([Zed][3])

它解决的正好是你以后一定会碰上的问题：

```text
byte offset
character offset
UTF-16 offset
line/column
屏幕坐标
Markdown AST position
```

这些坐标怎么转换？

Markdown 编辑器做到后面，**坐标系统是非常容易把人写崩的一部分**。

例如：

```text
鼠标点击
   ↓
屏幕 x/y
   ↓
visual line
   ↓
glyph
   ↓
UTF-8 byte offset
   ↓
Markdown source offset
   ↓
AST Node
```

Zed 非常值得你研究这个思想。

但注意一个问题：

**不要直接复制 Zed。**

它目前主要是 GPL-3.0-or-later，部分组件另有 Apache-2.0 标记。([GitHub][2])

所以：

> 学架构可以，复制代码要谨慎。

---

# 三、Xi Editor —— 已停止开发，但“编辑器理论”非常值得看

**优先级：★★★★★**

这个项目虽然已经停止继续开发，但我依然建议你读。

Xi 官方现在也明确表示项目 discontinued，并推荐 Lapce 作为某种精神继承者。([GitHub][4])

但是 Xi 留下来一个非常有价值的东西：

## Rope Science

它非常系统地讨论了：

> 大文本到底应该怎么存储和增量修改？

Xi 作者后来回顾项目时仍然认为 `xi-rope` 是 Xi 最有价值的成果之一。([Raph Levien’s blog][5])

你尤其应该理解：

```text
String
Piece Table
Gap Buffer
Rope
B-Tree Rope
CRDT Rope
```

之间的区别。

另外 Xi 曾经提出一个我认为特别适合你项目的思想：

> editor construction kit

也就是编辑器不应该是一个巨大 Editor 类，而应该拆成可以自由组合的组件。([GitHub][6])

例如：

```text
marklight-core/
    buffer
    selection
    transaction
    history
    command
    syntax
    layout
    decoration
```

而不是：

```text
MarkdownEditor.rs
5000 行
```

---

# 四、Ropey —— 你现在就可以考虑采用

**优先级：★★★★★**

这是 Rust 里面非常成熟的 Rope 实现。

官方定位就是：

> 用于文本编辑器等程序的 UTF-8 text rope。([GitHub][7])

我不建议你第一阶段自己实现 Rope。

直接：

```rust
ropey::Rope
```

就可以。

你的文档核心可以设计成：

```rust
pub struct Document {
    text: Rope,
    revision: Revision,
}
```

然后：

```rust
pub struct Change {
    from: TextOffset,
    to: TextOffset,
    insert: String,
}
```

---

# 五、CodeMirror 6 —— 我认为你最该“抄思想”的项目

**优先级：★★★★★+**

虽然是 TypeScript，但它对你可能比很多 Rust 项目价值更大。

CodeMirror 6 最重要的设计不是 DOM。

而是：

# State + Transaction + Extension

官方系统指南本身就是围绕 transaction 描述 document、selection 和其他 state 的变化。([CodeMirror][8])

核心思想：

```text
EditorState
     │
     │ Transaction
     ↓
EditorState'
     │
     ↓
EditorView
```

也就是说：

**UI 不是数据源。**

State 才是真实状态。

例如 Markdown：

```rust
struct EditorState {
    document: Document,
    selection: Selection,
    syntax: SyntaxState,
    history: History,
    extensions: Extensions,
}
```

操作：

```text
BoldCommand
     ↓

Transaction
{
    change: "**hello**"
}

     ↓

EditorState
```

以后 AI 修改：

```text
AI
 ↓
Transaction
 ↓
EditorState
```

插件修改：

```text
Plugin
 ↓
Transaction
 ↓
EditorState
```

Undo：

```text
Transaction
 ↓
History
```

这样整个系统会非常干净。

---

# 六、ProseMirror —— Markdown WYSIWYG 必须研究

**优先级：★★★★★**

如果你想做的是类似：

* Typora
* Obsidian Live Preview
* Milkdown

这种 Markdown 所见即所得编辑，那么 ProseMirror 基本绕不开。

它最值得研究的是：

```text
Schema
Document Model
Transaction
Selection
Step
Plugin
Decoration
```

ProseMirror 甚至有专门的 Markdown schema，以及 CommonMark / Markdown parser 和 serializer。([GitHub][9])

但这里有一个非常重要的区别。

ProseMirror 更倾向：

```text
Markdown
   ↓
Document Tree
   ↓
编辑 Document Tree
   ↓
Markdown
```

而我对你的项目更加推荐：

```text
Markdown Source ← 真相
      │
      ├── Syntax Tree
      ├── Render Tree
      └── View
```

原因是你做的是 **Markdown Editor**，不是普通 Rich Text Editor。

这样才能真正保持：

```md
**abc**
__abc__
```

这种原始 Markdown 差异。

否则 AST round-trip 很容易把用户原始格式吃掉。

---

# 二、Markdown 专项：这三个项目值得看

## 7. tree-sitter-markdown

**强烈推荐。**

Tree-sitter 本身就是为**增量解析**设计的，可以随着文本编辑高效更新语法树。([GitHub][10])

目前维护的 Markdown grammar 支持 CommonMark，并带有一些可选择的扩展。([GitHub][11])

Rust crate `tree_sitter_md` 更明确拆成：

```text
block grammar
inline grammar
```

两套 grammar。([Docs.rs][12])

这非常值得你研究，因为 Markdown 天然有：

```text
Block
 ├── Heading
 ├── Paragraph
 ├── BlockQuote
 ├── List
 ├── Table
 └── CodeBlock

Inline
 ├── Text
 ├── Emphasis
 ├── Strong
 ├── Code
 ├── Link
 └── Image
```

你的内部架构也应该有这个层级意识。

---

# 8. Comrak

如果你需要：

```text
Markdown → AST
Markdown → HTML
CommonMark
GFM
Table
Strikethrough
Task List
```

Comrak 很值得采用或者参考。

它是 Rust 的 CommonMark + GitHub Flavored Markdown parser/renderer，并默认兼容 CommonMark 0.31.2。([GitHub][13])

但是我会建议：

### Comrak 用于：

```text
Export
Preview
HTML generation
规范兼容测试
```

### Tree-sitter 用于：

```text
实时编辑
增量解析
syntax ranges
```

也就是：

```text
               ┌─ Tree-sitter → 实时语法
Markdown Source│
               └─ Comrak → Export / HTML
```

不要强行一个 Parser 干所有事情。

---

# 9. pulldown-cmark

也是非常值得参考的 Rust Markdown parser。

它是 pull parser，也就是：

```rust
Parser
 ↓
Event::Start(...)
Event::Text(...)
Event::End(...)
```

而不是一定构造完整 AST。([GitHub][14])

非常适合学习：

> Markdown streaming parsing。

但如果你的目标是编辑器，我倾向于：

**Comrak / Tree-sitter > pulldown-cmark**

因为编辑器需要更丰富的 node / range 信息。

---

# 三、另外几个“非 Rust，但是极有价值”的项目

## Lexical

Meta 的编辑器框架。

它特别值得学习：

```text
EditorState
Node
Selection
Command
Transform
```

Lexical 的 EditorState 由 Node Tree + Selection 构成。([lexical.dev][15])

而 `Command`：

```text
EnterCommand
TabCommand
DeleteCommand
FormatTextCommand
```

这种设计非常适合你。

例如：

```rust
enum EditorCommand {
    InsertText(String),
    DeleteBackward,
    InsertParagraph,
    ToggleBold,
    ToggleItalic,
    InsertLink,
}
```

不要：

```rust
if key == Enter ...
if key == Backspace ...
```

散落整个代码库。

---

# Milkdown

如果你想研究：

> Markdown 编辑器怎么插件化？

一定看看 Milkdown。

它自己就把定位写得非常明确：

> Plugin-driven WYSIWYG Markdown editor framework。([milkdown.dev][16])

它强调：

```text
syntax plugin
theme plugin
UI plugin
...
```

我认为这个思路特别适合你后面的 MarkLight：

```text
marklight-plugin-mermaid
marklight-plugin-katex
marklight-plugin-table
marklight-plugin-image
marklight-plugin-wikilink
marklight-plugin-frontmatter
marklight-plugin-ai
```

而不是不断往 `editor-core` 里加：

```rust
if mermaid...
if latex...
if table...
```

---

# 四、还有一个你未来如果做纯 Rust UI，非常值得看的：Parley

如果以后你决定：

> 不依赖 WebView，Rust 自己绘制文字。

那么建议研究：

**Linebender / Parley**

Parley 本身处理 rich text layout，而且已经提供 text selection / editing 相关 utility。([GitHub][17])

它负责这种事情：

```text
UTF-8 text
   ↓
font shaping
   ↓
line breaking
   ↓
glyph position
   ↓
x/y coordinates
```

即：

```text
"你好 Hello 👋"

↓

glyph
glyph
glyph
...
```

这层如果自己从零写，工作量非常恐怖。

所以：

> 编辑器核心可以自己造；
> 字体 shaping/layout 不要自己造。

---

# 五、我会怎么重新定义你的“从零”

我觉得这里需要稍微修正一下你的目标。

你说：

> 使用 Rust 从零做 Markdown 编辑器

我建议不要把“从零”理解成：

```text
Rope 自己写
Markdown parser 自己写
字体布局自己写
渲染自己写
光标自己写
Undo 自己写
```

这实际上是在做：

> 一个十年级别的文本编辑基础设施项目。

更合理的“从零”是：

# 自己设计 Editor Architecture

但是底层采用成熟组件。

例如：

```text
┌─────────────────────────────┐
│          MarkLight           │
├─────────────────────────────┤
│        Editor View           │
│ Vue / Native / GPU           │
├─────────────────────────────┤
│       Render Model           │
│ Decoration / Layout          │
├─────────────────────────────┤
│       Editor State           │
│ Document / Selection         │
│ Transaction / History        │
├─────────────────────────────┤
│       Markdown Layer         │
│ AST / Syntax / Mapping       │
├─────────────────────────────┤
│        Text Engine           │
│ Rope / Offset / Line Index   │
├─────────────────────────────┤
│        Infrastructure        │
│ Ropey / Tree-sitter          │
│ Comrak                       │
└─────────────────────────────┘
```

---

# 六、我尤其建议你重新考虑一个核心架构

你以后一定会遇到一个问题：

> Markdown AST 到底是不是 Editor 的核心数据？

我建议：

**不是。**

一定让：

# Markdown Source 是唯一真相

```text
                 ┌── Syntax Tree
                 │
                 ├── Render Tree
Rope / Markdown ─┼── Outline
                 │
                 ├── Search Index
                 │
                 └── Export AST
```

而不要：

```text
Markdown
 ↓
AST
 ↓
编辑 AST
 ↓
重新生成 Markdown
```

因为后者很容易导致：

```md
# title
```

变：

```md
# title

```

或者：

```md
-   aaa
```

自动变：

```md
- aaa
```

甚至 comment、空格、换行习惯全部改变。

对于真正的 Markdown 编辑器：

> **源码应该始终保持 first-class citizen。**

这是我认为你现在最值得尽早确定的一项设计。

---

# 七、如果只允许你研究 5 个，我推荐这个顺序

| 顺序 | 项目               | 学什么                             |
| -- | ---------------- | ------------------------------- |
| ①  | **CodeMirror 6** | State / Transaction / Extension |
| ②  | **Helix**        | Rust Editor Core                |
| ③  | **ProseMirror**  | Document Model / WYSIWYG        |
| ④  | **Zed**          | Rope / SumTree / Layout / GPU   |
| ⑤  | **Milkdown**     | Markdown + Plugin Architecture  |

然后具体实现依赖：

```text
Ropey
+
Tree-sitter Markdown
+
Comrak
```

---

# 八、我认为你现在开发慢的真正原因

从你之前 MarkLight 的目标看，你已经涉及 WYSIWYG、Mermaid、KaTeX、公众号排版、图片本地化等能力。([GitHub][18])

问题很可能已经不是 Rust 熟不熟，而是：

```text
功能
 ↓
直接实现
 ↓
发现架构不够
 ↓
重构
 ↓
继续加功能
 ↓
又发现抽象不够
```

所以会感觉：

> 每一个功能都能做，但是每个功能做起来都特别累。

现在更值得做的是暂停继续增加 Editor Feature，先把：

```text
Document
Selection
Transaction
Command
History
Syntax
Decoration
Plugin
```

这 **8 个核心概念**定义下来。

一旦它们稳定以后：

```text
Bold
Italic
Heading
Table
Mermaid
KaTeX
AI
公众号样式
```

都只是往这个体系里面加东西，而不是每次重新设计一遍编辑器。

---

我目前最推荐你的路线其实可以概括成一句：

> **Helix 的 Rust Core + CodeMirror 6 的 State/Transaction + ProseMirror 的文档编辑思想 + Milkdown 的插件系统 + Zed 的高性能数据结构思想。**

底层先用：

```text
Ropey
Tree-sitter
Comrak
```

而不是所有东西自己造。

如果你愿意继续推进这个项目，下一步其实非常适合直接做一份 **《MarkLight Editor Core Architecture v2》**，把 `Document / Transaction / Selection / Command / Syntax / Decoration / Plugin / Renderer` 的 Rust trait 和模块目录都设计出来，这样你可以直接把文档交给 Codex/Claude 开始按架构重构，而不是继续边写边想。

[1]: https://github.com/helix-editor/helix/blob/master/docs/architecture.md?utm_source=chatgpt.com "helix/docs/architecture.md at master · helix-editor/helix"
[2]: https://github.com/zed-industries/zed?utm_source=chatgpt.com "zed-industries/zed: Code at the speed of ..."
[3]: https://zed.dev/blog/hiring?utm_source=chatgpt.com "Hiring at Zed: Building in Real Time — Zed's Blog"
[4]: https://github.com/xi-editor/xi-editor?utm_source=chatgpt.com "Xi editor"
[5]: https://raphlinus.github.io/xi/2020/06/27/xi-retrospective.html?utm_source=chatgpt.com "xi-editor retrospective"
[6]: https://github.com/xi-editor/xi-editor/issues/1187?utm_source=chatgpt.com "Towards a text editor construction kit · Issue #1187"
[7]: https://github.com/cessen/ropey?utm_source=chatgpt.com "cessen/ropey: A utf8 text rope for manipulating and editing ..."
[8]: https://codemirror.net/docs/guide/?utm_source=chatgpt.com "CodeMirror System Guide"
[9]: https://github.com/ProseMirror/prosemirror-markdown?utm_source=chatgpt.com "ProseMirror Markdown integration"
[10]: https://github.com/topics/tree-sitter?utm_source=chatgpt.com "tree-sitter · GitHub Topics"
[11]: https://github.com/tree-sitter-grammars/tree-sitter-markdown?utm_source=chatgpt.com "Markdown grammar for tree-sitter"
[12]: https://docs.rs/tree-sitter-md?utm_source=chatgpt.com "tree_sitter_md - Rust"
[13]: https://github.com/kivikakk/comrak?utm_source=chatgpt.com "kivikakk/comrak: CommonMark + GFM compatible ..."
[14]: https://github.com/pulldown-cmark/pulldown-cmark?utm_source=chatgpt.com "pulldown-cmark"
[15]: https://lexical.dev/docs/intro?utm_source=chatgpt.com "Introduction"
[16]: https://milkdown.dev/?utm_source=chatgpt.com "Milkdown"
[17]: https://github.com/linebender/parley?utm_source=chatgpt.com "linebender/parley: Rich text layout library"
[18]: https://github.com/zed-industries/zed/issues/15066?utm_source=chatgpt.com "Edit Markdown with Live Preview in a Single View #15066"
