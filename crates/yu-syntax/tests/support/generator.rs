//! 生成 Markdown 文档的语料生成器。
//!
//! 两个入口：
//!
//! - [`deviation_free_document`]：**刻意避开**不变量第 F 节的三条已登记偏差，
//!   因此可以拿 comrak 当 oracle 直接比 HTML；
//! - [`arbitrary_document`]：什么都可能出现，只用来验结构性质。
//!
//! 生成器是纯函数：同一个种子永远得到同一份文档。这是它能进确定性门禁的
//! 前提，也是 fuzz 找到失败之后能被复现的前提。

/// xorshift64。要的是可复现和分布均匀，不是密码学强度。
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // 种子为 0 会让 xorshift 卡死在 0。
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            usize::try_from(self.next() % bound as u64).unwrap_or(0)
        }
    }

    fn pick<'a, T>(&mut self, choices: &'a [T]) -> &'a T {
        &choices[self.below(choices.len())]
    }

    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in.max(1)) == 0
    }
}

const WORDS: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "text", "value", "code", "line", "note", "羽", "編集",
    "café", "naïve",
];

const INLINE_TEMPLATES: &[&str] = &[
    "{w}",
    "*{w}*",
    "**{w}**",
    "_{w}_",
    "__{w}__",
    "`{w}`",
    "``{w} ` {w}``",
    "[{w}](/{w})",
    "[{w}](/{w} \"{w}\")",
    "![{w}](/{w})",
    "<https://example.com/{w}>",
    "<user@example.com>",
    "&amp; &#65; &#x41;",
    "\\* \\_ \\[ \\\\",
    "{w} <span>{w}</span>",
    "{w}  ",
    "{w}\\",
];

fn inline(rng: &mut Rng, out: &mut String) {
    let count = 1 + rng.below(3);
    for index in 0..count {
        if index > 0 {
            out.push(' ');
        }
        let template = *rng.pick(INLINE_TEMPLATES);
        for part in template.split("{w}") {
            out.push_str(part);
            out.push_str(rng.pick(WORDS));
        }
        // 上面的循环在末尾多补了一个词，去掉它以免模板尾部走样。
        for word in WORDS {
            if out.ends_with(word) {
                out.truncate(out.len() - word.len());
                break;
            }
        }
    }
}

/// 一份不触发已登记偏差的文档。
///
/// 三条回避：
///
/// - **不出制表符**（F2）。缩进一律用空格。
/// - **链接里不嵌套方括号**（F1）。行内链接的文字里没有 `[`，
///   引用式链接的标签与定义严格配对，于是每一个 `[` 都会成立。
/// - **引用标签只用 ASCII 小写**（F3）。
pub fn deviation_free_document(seed: u64) -> String {
    let mut rng = Rng::new(seed);
    let mut out = String::new();
    let blocks = 1 + rng.below(8);
    let mut reference_count = 0_usize;

    for _ in 0..blocks {
        match rng.below(12) {
            0 => {
                let level = 1 + rng.below(6);
                out.push_str(&"#".repeat(level));
                out.push(' ');
                inline(&mut rng, &mut out);
                out.push('\n');
            }
            1 => {
                inline(&mut rng, &mut out);
                out.push('\n');
                if rng.chance(2) {
                    inline(&mut rng, &mut out);
                    out.push('\n');
                }
            }
            2 => {
                inline(&mut rng, &mut out);
                out.push('\n');
                out.push_str(if rng.chance(2) { "===\n" } else { "---\n" });
            }
            3 => {
                let items = 1 + rng.below(3);
                let marker = *rng.pick(&["-", "*", "+"]);
                for _ in 0..items {
                    out.push_str(marker);
                    out.push(' ');
                    inline(&mut rng, &mut out);
                    out.push('\n');
                    if rng.chance(3) {
                        out.push_str("  ");
                        out.push_str(marker);
                        out.push(' ');
                        inline(&mut rng, &mut out);
                        out.push('\n');
                    }
                }
            }
            4 => {
                let items = 1 + rng.below(3);
                for index in 0..items {
                    out.push_str(&format!("{}. ", index + 1));
                    inline(&mut rng, &mut out);
                    out.push('\n');
                }
            }
            5 => {
                let lines = 1 + rng.below(3);
                for _ in 0..lines {
                    out.push_str("> ");
                    inline(&mut rng, &mut out);
                    out.push('\n');
                }
            }
            6 => {
                let fence = *rng.pick(&["```", "~~~", "````"]);
                out.push_str(fence);
                if rng.chance(2) {
                    out.push_str("rust");
                }
                out.push('\n');
                for _ in 0..1 + rng.below(3) {
                    out.push_str(rng.pick(WORDS));
                    out.push_str(" *not emphasis*\n");
                }
                out.push_str(fence);
                out.push('\n');
            }
            7 => {
                for _ in 0..1 + rng.below(2) {
                    out.push_str("    ");
                    out.push_str(rng.pick(WORDS));
                    out.push_str(" indented\n");
                }
            }
            8 => out.push_str(rng.pick(&["---\n", "***\n", "___\n", "* * *\n"])),
            9 => {
                // 引用定义与用它的段落成对出现，标签保证能匹配上。
                let label = format!("ref{reference_count}");
                reference_count += 1;
                out.push_str(&format!("[{label}]: /target/{label} \"title\"\n\n"));
                out.push_str(&format!("see [{label}] and [text][{label}].\n"));
            }
            11 => {
                // GFM 任务项。标记后固定一个空格、列表保持紧凑：多余的空白
                // 与松散列表由 `task_lists_match_comrak` 逐条压着，这里只
                // 负责让「任务项与别的语法混在一份文档里」被大量样本走到。
                let items = 1 + rng.below(3);
                let marker = *rng.pick(&["-", "*", "+"]);
                for _ in 0..items {
                    out.push_str(marker);
                    out.push_str(if rng.chance(2) { " [x] " } else { " [ ] " });
                    inline(&mut rng, &mut out);
                    out.push('\n');
                }
            }
            _ => {
                out.push_str("<div>\n");
                out.push_str(rng.pick(WORDS));
                out.push_str("\n</div>\n");
            }
        }
        out.push('\n');
    }
    out
}

/// 一份什么都可能出现的文档。只用来验结构性质，不比 HTML。
///
/// 与上面那份不同，这里**故意**制造畸形：不闭合的围栏、悬空的方括号、
/// 制表符、乱序的容器标记、控制字符。不变量 C5 要求这些输入都不得丢字节、
/// 不得凭空造节点。
pub fn arbitrary_document(seed: u64) -> String {
    let mut rng = Rng::new(seed);
    let mut out = String::new();
    let pieces = 1 + rng.below(24);

    const FRAGMENTS: &[&str] = &[
        "#",
        "##",
        "###",
        "####### ",
        "\t",
        "  ",
        "    ",
        "\n",
        "\r\n",
        "> ",
        ">",
        ">>",
        "- ",
        "-",
        "* ",
        "+ ",
        "1. ",
        "1)",
        "99999999999. ",
        "```",
        "~~~",
        "``",
        "`",
        "    ",
        "---",
        "***",
        "___",
        "[",
        "]",
        "](",
        ")",
        "[a]",
        "[a][",
        "[]",
        "![",
        "<",
        ">",
        "<div>",
        "</div>",
        "<!--",
        "-->",
        "<?",
        "?>",
        "<![CDATA[",
        "]]>",
        "&",
        "&amp;",
        "&#",
        "&#x",
        ";",
        "\\",
        "\\\\",
        "*",
        "**",
        "***",
        "_",
        "__",
        "|",
        "text",
        "羽",
        "🙂",
        "\u{0}",
        "\u{feff}",
        "\u{2028}",
        "a",
        " ",
    ];

    for _ in 0..pieces {
        out.push_str(rng.pick(FRAGMENTS));
        if rng.chance(4) {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{arbitrary_document, deviation_free_document};

    /// 生成器必须是纯的：同种子同结果。它进确定性门禁、fuzz 失败能复现，
    /// 都靠这一条。
    #[test]
    fn generation_is_reproducible() {
        for seed in [1_u64, 42, 0xDEAD_BEEF, u64::MAX] {
            assert_eq!(deviation_free_document(seed), deviation_free_document(seed));
            assert_eq!(arbitrary_document(seed), arbitrary_document(seed));
        }
        assert_ne!(deviation_free_document(1), deviation_free_document(2));
    }

    /// 回避条款要真的生效，否则「差异只可能是 Yu 的 bug」这句话不成立。
    #[test]
    fn deviation_free_documents_avoid_the_registered_deviations() {
        for seed in 0..500_u64 {
            let document = deviation_free_document(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            assert!(!document.contains('\t'), "F2：不该出现制表符\n{document}");
            for line in document.lines() {
                // F1：任何**未转义**的 `[` 都必须在同一行闭合，且中间不再有
                // `[`。`\[` 不开链接，不受这条约束。
                let bytes = line.as_bytes();
                let mut index = 0_usize;
                let mut open_brackets = 0_usize;
                while index < bytes.len() {
                    match bytes[index] {
                        b'\\' => index += 1,
                        b'[' => {
                            open_brackets += 1;
                            assert!(open_brackets <= 1, "F1：方括号嵌套\n{line}");
                        }
                        b']' => open_brackets = open_brackets.saturating_sub(1),
                        _ => {}
                    }
                    index += 1;
                }
                assert_eq!(open_brackets, 0, "F1：未闭合的 `[`\n{line}");
            }
        }
    }
}
