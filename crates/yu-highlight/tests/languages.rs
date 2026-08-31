//! 着色的判据。
//!
//! **判据落在源码文本上，不落在着色器自己的记账上**：每一条断言都把
//! `code[span.start..span.end]` 切出来跟一个字面量比，所以「区间算错了」与
//! 「角色判错了」是两件分得开的事。数条数压不住指错了哪一处——第四刀在
//! 「当前命中」上为这件事付过两次。

use yu_core::TextRole;
use yu_highlight::{Highlighter, Language, RoleSpan};

/// 切出一条区间覆盖的文本。
fn text_of(code: &str, span: RoleSpan) -> &str {
    &code[span.start..span.end]
}

/// 一段代码里角色为 `role` 的全部文本。
fn texts_with_role<'a>(code: &'a str, spans: &[RoleSpan], role: TextRole) -> Vec<&'a str> {
    spans
        .iter()
        .filter(|span| span.role == role)
        .map(|span| text_of(code, *span))
        .collect()
}

/// 每一条产出都必须满足的形状。
///
/// 这几条不是装饰性的：越界或落在字符中间会让 `TextRange::new` 之后的装饰指
/// 到半个字上，而 `extension_decorations.rs` 的结构性扫描只查装饰不越**块**，
/// 查不出「切在一个汉字中间」。
fn assert_well_formed(code: &str, spans: &[RoleSpan]) {
    let mut previous_end = 0_usize;
    for span in spans {
        assert!(span.start < span.end, "区间非空：{span:?}");
        assert!(span.start >= previous_end, "有序且互不重叠：{span:?}");
        assert!(span.end <= code.len(), "不越界：{span:?} / {}", code.len());
        assert!(
            code.is_char_boundary(span.start) && code.is_char_boundary(span.end),
            "两端都在字符边界上：{span:?}"
        );
        assert_ne!(span.role, TextRole::Plain, "Plain 不回报：{span:?}");
        previous_end = span.end;
    }
}

/// 每一种语言都要真的着上色，而且着到**指定的那一段**上。
///
/// 这条压的是「加了一个 `Language` 变体，忘了给它接 grammar 或别名」——那种
/// 漏法不报错，只是那种语言的代码块永远是一片正文色。
///
/// 每份语料自带一条 `(角色, 文本)` 期望，而不是共用一个「有注释就算过」：
/// **JSON 没有注释**，共用判据会逼着语料去迁就判据。
#[test]
fn every_registered_language_highlights_its_own_marker() {
    let fixtures: &[(Language, &str, &str, TextRole, &str)] = &[
        (
            Language::Bash,
            "bash",
            "# c\nfor i in 1 2; do echo x; done\n",
            TextRole::Keyword,
            "for",
        ),
        (
            Language::JavaScript,
            "js",
            "const a = 1; // c\nfunction f(b) { return b; }\n",
            TextRole::Keyword,
            "const",
        ),
        (
            Language::Json,
            "json",
            "{ \"a\": [1, true, null] }\n",
            TextRole::Constant,
            "true",
        ),
        (
            Language::Python,
            "py",
            "def f(x):\n    # c\n    return None\n",
            TextRole::Keyword,
            "def",
        ),
        (
            Language::Rust,
            "rust",
            "fn main() {\n    // c\n    let x: u32 = 1;\n}\n",
            TextRole::Type,
            "u32",
        ),
    ];
    assert_eq!(
        fixtures.len(),
        Language::ALL.len(),
        "每一种登记的语言都要有一份语料"
    );
    let highlighter = Highlighter::new();
    for (language, info, code, role, marker) in fixtures {
        assert_eq!(
            Language::from_info(info),
            Some(*language),
            "别名 {info} 认到 {}",
            language.name()
        );
        let spans = highlighter.spans(*language, code);
        assert_well_formed(code, &spans);
        assert!(
            texts_with_role(code, &spans, *role).contains(marker),
            "{} 的 {marker:?} 没被认成 {role:?}：{spans:?}",
            language.name()
        );
    }
}

/// 角色指到了**哪一段**，不只是「有几条」。
#[test]
fn roles_land_on_the_right_source_text() {
    let code = "fn main() {\n    // hi\n    let x: u32 = 1;\n    println!(\"a\");\n}\n";
    let spans = Highlighter::new().spans(Language::Rust, code);
    assert_well_formed(code, &spans);
    assert_eq!(
        texts_with_role(code, &spans, TextRole::Keyword),
        vec!["fn", "let"]
    );
    assert_eq!(
        texts_with_role(code, &spans, TextRole::Comment),
        vec!["// hi"]
    );
    assert_eq!(texts_with_role(code, &spans, TextRole::Type), vec!["u32"]);
    assert_eq!(
        texts_with_role(code, &spans, TextRole::Literal),
        vec!["\"a\""]
    );
    assert_eq!(
        texts_with_role(code, &spans, TextRole::Function),
        vec!["main", "println!"]
    );
}

/// 嵌套的 capture 取**最里面**那一层，不是最外面那一层。
///
/// `tree-sitter-highlight` 的事件流是一个栈，栈顶是最里面那一层。取错一端不
/// 报错，只是把整段模板串染成字符串色——而**浅层语料压不住这一条**：栈里只有
/// 一层时两端是同一个值。上面那几份语料都是浅的，所以这一条要自己造深的。
///
/// 一处内插就是一个两层结构：外层是整段字符串，里面套着定界符与被插进去的
/// 那个名字。
#[test]
fn nested_captures_resolve_to_the_innermost_one() {
    let highlighter = Highlighter::new();
    for (language, code, needle, role) in [
        (
            Language::JavaScript,
            "const s = `x${name}y`;\n",
            "name",
            TextRole::Variable,
        ),
        (
            Language::Python,
            "s = f\"a{name}b\"\n",
            "name",
            TextRole::Variable,
        ),
        (
            Language::Bash,
            "echo \"$HOME/x\"\n",
            "HOME",
            TextRole::Variable,
        ),
    ] {
        let spans = highlighter.spans(language, code);
        assert_well_formed(code, &spans);
        // 外层确实盖着它：同一段代码里还有被认成字符串的部分，说明这里真的是
        // 一个「字符串里套着别的东西」的两层结构，不是一段裸标识符。
        assert!(
            spans.iter().any(|span| span.role == TextRole::Literal),
            "{} 的语料没有外层字符串，压不住嵌套：{spans:?}",
            language.name()
        );
        assert!(
            texts_with_role(code, &spans, role).contains(&needle),
            "{} 里的 {needle:?} 没被认成 {role:?}——取的可能是栈底那一层：{spans:?}",
            language.name()
        );
    }
}

/// 多字节字符两侧都不切错。
///
/// tree-sitter 给的是字节偏移，而语料里一旦出现中文，「差一个字符」与
/// 「差一个字节」就不再是同一件事。切在半个汉字上不 panic——`&str` 的索引会
/// panic，但那是在**调用方**那边，这里只回报数字。
#[test]
fn multibyte_source_keeps_char_boundaries() {
    let code = "let 中文变量 = \"你好，世界\"; // 中文注释\n";
    let spans = Highlighter::new().spans(Language::Rust, code);
    assert_well_formed(code, &spans);
    assert_eq!(
        texts_with_role(code, &spans, TextRole::Literal),
        vec!["\"你好，世界\""]
    );
    assert_eq!(
        texts_with_role(code, &spans, TextRole::Comment),
        vec!["// 中文注释"]
    );
}

/// 三条降级：认不出语言、空代码、语法错的代码。
///
/// 三种都必须回报**空或合法**，不能 panic、不能给出畸形区间。着色是这条路上
/// 唯一可以「什么都不做」的一层（overview-v2 第 6.2 节：fenced code 内部高亮
/// 错误无害），所以它的失败必须是安静的、结构完好的。
#[test]
fn degrades_quietly_on_unknown_empty_and_broken_input() {
    assert_eq!(Language::from_info("brainfuck"), None);
    let highlighter = Highlighter::new();
    for code in ["", "\n", "   \n\n"] {
        assert!(
            highlighter.spans(Language::Rust, code).is_empty(),
            "空代码不着色：{code:?}"
        );
    }
    for (language, code) in [
        (Language::Rust, "fn broken( { { {"),
        (Language::Json, "not json at all }}}"),
        (Language::Python, "def :::\n  ???\n"),
        (Language::Bash, "for do done fi ;;; ((("),
        (Language::JavaScript, "function ((( {{{ ```"),
    ] {
        let spans = highlighter.spans(language, code);
        assert_well_formed(code, &spans);
    }
}

/// memo 不许改变答案。
///
/// 它是**代价**的门不是正确性的门（`Highlighter` 的类型文档写了为什么），
/// 而证明这一点的唯一办法是让同一个问题走两条路：一个用过的着色器 vs 一个
/// 全新的。判据不来自被测的那条路——新着色器的 memo 是空的。
#[test]
fn the_memo_cannot_change_the_answer() {
    // **同一种语言的两份语料必须结构不同。** 第一版用的是
    // `fn a() { let x = 1; }` 与 `fn b() { let y = 2; }`——它们的 `RoleSpan`
    // 逐个字节完全相同（标识符都是一个字符），于是「memo 的键里没有代码文本」
    // 这个变异活了下来：拿上一份的答案回报，比出来一模一样。
    let cases: &[(Language, &str)] = &[
        (Language::Rust, "fn a() { let x = 1; }\n"),
        (Language::Rust, "// 只有一行注释\n"),
        (
            Language::Rust,
            "struct VeryLongName { field: Vec<String> }\n",
        ),
        (Language::Json, "{ \"k\": 1 }\n"),
        (Language::Rust, "fn a() { let x = 1; }\n"),
    ];
    let warm = Highlighter::new();
    // 先把 memo 填成别的东西，确保后面每一次都要么命中要么替换。
    let _ = warm.spans(Language::Python, "x = 1\n");
    for (language, code) in cases {
        let cold = Highlighter::new();
        assert_eq!(
            warm.spans(*language, code),
            cold.spans(*language, code),
            "{} / {code:?}",
            language.name()
        );
        // 同一个问题连问两次也一样——第二次一定走 memo。
        assert_eq!(warm.spans(*language, code), warm.spans(*language, code));
        // 中间插一份**同语言但不同内容**的，再问回来：memo 的键里少了代码
        // 文本的话，这里拿到的会是中间那一份的答案。
        let _ = warm.spans(*language, "\n\n// 插进来的一份，够长够不一样\n\n");
        assert_eq!(warm.spans(*language, code), cold.spans(*language, code));
    }
}

/// 同一段文本换一种语言必须换一个答案。
///
/// memo 的键里少了语言那一半的表现就是这一条：在 JSON 块后面紧跟一个内容
/// 相同的 Rust 块，第二个块拿到第一个块的颜色。
#[test]
fn the_memo_keys_on_the_language_too() {
    let code = "{ \"a\": 1 }\n";
    let highlighter = Highlighter::new();
    let as_json = highlighter.spans(Language::Json, code);
    let as_rust = highlighter.spans(Language::Rust, code);
    assert_ne!(as_json, as_rust, "同一段文本两种语言不该是同一个答案");
    // 再问一遍 JSON：memo 现在装着 Rust 那一份。
    assert_eq!(highlighter.spans(Language::Json, code), as_json);
}
