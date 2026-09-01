//! 导出与导入是一对，这里压住那个「对」。
//!
//! # 为什么判据是这两条，而不是「导出的 HTML 等于某个字面量」
//!
//! S7 第六刀把 HTML 导出换成了 comrak。原来那 12 条逐字节断言如果原地改成
//! comrak 的新输出，就是**在断言一个第三方库的行为**：永远绿，什么都不证明。
//! 这里的两条都不问 comrak 输出了什么字节——
//!
//! 1. **自家导入器接得住自家导出。** 判据在 `html_import` 那一侧，它是 Yu
//!    自己写的、与 comrak 无关的一份实现。这一条压住的是这一刀最容易静默
//!    破掉的东西：**Yu 里 ⌘C 再 ⌘V 降级成纯文本**，没有任何报错。
//!    实测过这不是空话：换 comrak 之前导入器接受 646/652 条规范用例的导出，
//!    之后掉到 491/652。放宽了三处（`<hr>`、`title`、`align=`）之后，Yu 自己
//!    语法集合内的那部分才重新接得住。
//! 2. **走一圈之后是不动点。** `导出 → 导入 → 再导出` 必须与第一次导出逐字节
//!    相同。它比「导入回来等于原文」弱，也正因此才是对的：Markdown 有很多种
//!    写法表示同一件事（`*a*` 与 `_a_`、`1)` 与 `1.`），要求原样回来是在要求
//!    导入器复刻源码的拼法。**不动点要求的是两边对语义的理解一致**，判据是
//!    两个 Yu 自己的函数彼此相符，仍然不问 comrak 的字节。
//!
//! # 语料是手写的，覆盖 Yu 自己认得的语法
//!
//! 不用 CommonMark 的 652 条：那份语料里有大量原始 HTML，而导入器**按设计**
//! 拒绝原始 HTML（导出的是用户自己的文档，导入的是别人的 HTML，这个不对称
//! 是有意的）。拿它当语料只会逼这条断言去迁就一个不该成立的性质。

use yu_export::{export_html_fragment, import_html_fragment};

/// Yu 自己认得的每一种语法，各至少一条。
///
/// 加一种语法（`BlockKind` 多一个变体、多开一个 comrak 扩展）就要在这里加
/// 一行——否则新语法的「拷出去粘回来」一条断言都没有。
const YU_SYNTAX: &[&str] = &[
    // 段落与行内。
    "普通段落\n",
    "**粗** 与 *斜* 与 `代码`\n",
    "[文字](https://example.com)\n",
    "[带标题](https://example.com \"标题\")\n",
    "![替代](https://example.com/a.png)\n",
    "硬换行  \n第二行\n",
    "转义 \\*不是强调\\*\n",
    // 标题：ATX 与 Setext 两种拼法。
    "# 一级\n",
    "###### 六级\n",
    "## 收尾串 ##\n",
    "Setext 一级\n===\n",
    "Setext 二级\n---\n",
    // 围栏代码块，带语言名与不带。
    "```rust\nfn main() {}\n```\n",
    "```\n<&>\n```\n",
    // 引用块。
    "> 引用\n",
    "> 引用第一行\n> 第二行\n",
    // 列表：无序、有序、指定起点、嵌套、任务项。
    "- 一\n- 二\n",
    "1. 一\n2. 二\n",
    "3. 从三起\n4. 四\n",
    // `0.` 是合法的 CommonMark（起点九位以内即可），comrak 因此发
    // `start="0"`。**这一行是变异验证补的**：没有它，导入器那条「起点必须
    // 严格为正」的旧校验改回去一条用例都不红——判据本来就落在这条路上，是
    // 语料造不出差别。
    "0. 从零起\n1. 一\n",
    "- 外\n  - 内\n",
    "- [x] 做完\n- [ ] 没做\n",
    // 表格，四种对齐都要有。
    "| a | b | c | d |\n| --- | :--- | :---: | ---: |\n| 1 | 2 | 3 | 4 |\n",
    // 主题分隔线：三种拼法都是同一件事。
    "***\n",
    "---\n",
    "___\n",
    // 引用定义 + 引用式链接。
    "[文字][标签]\n\n[标签]: /目标\n",
    // 混排。
    "# 标题\n\n段落里有 [链接](/u)。\n\n- [x] 项\n\n```js\nlet a = 1;\n```\n\n---\n\n> 尾\n",
];

/// Yu 拷出去的 HTML，Yu 自己必须粘得回来。
#[test]
fn yu_accepts_every_fragment_it_exports() {
    for source in YU_SYNTAX {
        let html = export_html_fragment(source);
        let imported = import_html_fragment(&html).unwrap_or_else(|error| {
            panic!("自家导出粘不回来：{error}\n--- markdown ---\n{source}\n--- html ---\n{html}")
        });
        assert!(
            !imported.trim().is_empty(),
            "{source:?} 导入回来是空的——接受了但什么都没剩下，比拒绝更糟"
        );
    }
}

/// 走一圈之后是不动点：导出 → 导入 → 再导出，两次导出逐字节相同。
#[test]
fn export_import_export_is_a_fixed_point() {
    for source in YU_SYNTAX {
        let first = export_html_fragment(source);
        let imported = import_html_fragment(&first)
            .unwrap_or_else(|error| panic!("{source:?} 导入失败：{error}"));
        let second = export_html_fragment(&imported);
        assert_eq!(
            first, second,
            "{source:?} 走一圈之后语义变了\n--- 第一次 ---\n{first}\n--- 回来的 markdown ---\n{imported}\n--- 第二次 ---\n{second}"
        );
    }
}

/// 带语义的原始 HTML 仍然是这对里**有意**的那处不对称。
///
/// 导出照原样发（另一条路是 comrak 的 `<!-- raw HTML omitted -->`，那是静默
/// 删掉用户自己写的内容）；导入拒绝（那是别人的 HTML）。所以带原始 HTML 的
/// 文档**走不完这一圈**，而这是对的。
#[test]
fn semantic_raw_html_deliberately_does_not_round_trip() {
    for source in [
        "段落里有 <b>标签</b>\n",
        "<article>整块</article>\n",
        "<iframe src=\"https://example.com\"></iframe>\n",
    ] {
        let html = export_html_fragment(source);
        assert!(
            import_html_fragment(&html).is_err(),
            "{source:?} 的原始 HTML 不该被导入策略接受：{html}"
        );
    }
}

/// **纯呈现的容器不再拒，而是拍平——这是 S7 第七刀 c 的 G 节验收之后翻的案。**
///
/// 原来这条与上面那条是同一个用例，`<div>raw</div>` 也在名单里，理由是
/// 「那是别人的 HTML」。G 节实测下来这条理由在 `<div>` 上不成立：
/// **每一个真实浏览器发的剪贴板 HTML 都带 `<div>` / `<span>`**（Chrome 对一个
/// 连一个 `div` 都没有的页面也会把词间空白包成 `<span> </span>`），于是
/// 「拒绝 `<div>`」在实践上等于**拒绝每一次浏览器粘贴**——这个导入器上线以来
/// 从没有接住过一次，而那正是它存在的理由。
///
/// **代价是真的，写在这里**：用户自己写在 Markdown 里的 `<div>raw</div>`
/// 现在导入时被拍平成 `raw`。Yu 分不出「用户写的 div」与「浏览器的 div」
/// ——而后者是唯一真实的输入来源。**这条用例钉的就是这个代价**，免得下一个人
/// 把它当成缺口再翻回去。
///
/// 注意 Yu → Yu 不走这条路：剪贴板上有 canonical 的 Markdown flavor，
/// 平台侧的偏好顺序是 Markdown > 纯文本 > HTML（`--clipboard-self-check`）。
#[test]
fn presentational_containers_are_flattened_not_rejected() {
    assert_eq!(
        import_html_fragment(&export_html_fragment("<div>raw</div>\n")),
        Ok("raw\n".to_owned())
    );
    assert_eq!(
        import_html_fragment("<div><p>一段</p></div>"),
        Ok("一段".to_owned())
    );
    assert_eq!(
        import_html_fragment("<p>词<span> </span>之间</p>"),
        Ok("词 之间".to_owned())
    );
}

/// **判据来自一次真实的浏览器拷贝，不是我编的一段 HTML。**
///
/// `corpus/chrome-clipboard.html` 是 Chrome 138 在 macOS 26.5 上，对一个
/// 连一个 `<div>` 都没有的纯语义页面（一个 `<h2>`、一个带 `<strong>` 与
/// `<a>` 的 `<p>`、一个两项的 `<ul>`）按 ⌘A ⌘C 之后放在 `public.html` 上的
/// **原样字节**（S7 第七刀 c 的 G 节验收采集）。
///
/// 它一次性含着三样以前各自能让整段被拒的东西：开头的 `<meta charset>`、
/// 每个块元素上二十来条声明的 `style`、以及把词间空白包起来的
/// `<span> </span>`。**这三样都不是我造的语料想出来的**——正是因为造不出来，
/// 这个导入器上线以来从没有接住过一次浏览器粘贴。
#[test]
fn a_real_browser_clipboard_payload_imports_to_markdown() {
    let html = include_str!("corpus/chrome-clipboard.html");
    let markdown = import_html_fragment(html).expect("真实浏览器载荷必须接得住");

    // `\u{a0}` 是**不换行空格**，不是普通空格：源码页面那几处写的是 U+0020，
    // 是 Chrome 在拷贝时把它们换成了 NBSP（它要保住渲染出来的空白，好粘进
    // 别的富文本编辑器）。
    //
    // **不改写它，而是在这里点名。** 导入器的职责是翻译结构，不是重写文本；
    // 把 NBSP 悄悄换回空格会连着毁掉网页作者**故意**写的那些（数字与单位之
    // 间的 `&nbsp;`）。代价是粘进来的段落里夹着看不见的 NBSP——搜索匹配不上，
    // 已登记在 invariants 的 F 节。这条断言的价值正是**让它看得见**。
    assert_eq!(
        markdown,
        "## 网页里的标题\n\n\
         一段带\u{a0}**粗体**\u{a0}与\u{a0}[链接 A](https://example.com/a)\u{a0}的文字。\n\n\
         - 第一项\n- 第二项\u{a0}*斜体*"
    );
}
