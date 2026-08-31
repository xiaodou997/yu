//! CommonMark 0.31.2 的 652 条规范用例。
//!
//! 不变量 C7：CommonMark 语义以官方 spec 用例为准，任何有意偏差必须逐条
//! 登记进 `docs/specs/invariants.md` 第 F 节，未登记的偏差是 bug。
//!
//! # 这份测试防的两件事
//!
//! 「差分测试通过了但其实用例没跑到」是这一轮点名的危险类别。三道闸门：
//!
//! 1. **数量核对。** 用例数必须正好是 652，读文件失败会让测试红而不是跳过。
//! 2. **校验和核对。** `spec.json` 的 SHA-256 必须与登记值一致，规范换版
//!    只能是一次显式提交。
//! 3. **偏差登记表必须是紧的。** 表里的用例如果**通过了**，同样判失败——
//!    否则一条已经修好的偏差会永远留在表里，把「有意偏差」和「真 bug」重新
//!    混在一起。

#[path = "support/html.rs"]
mod html;

use std::collections::BTreeMap;
use std::path::PathBuf;

use yu_syntax::parse;

const EXPECTED_EXAMPLE_COUNT: usize = 652;
const SPEC_SHA256: &str = "d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20";

/// 原始通过率的棘轮：逐字节匹配规范期望输出的用例数不得少于这个值。
///
/// **这是个只能往上调的数字。** 它比一条 99% 的百分比阈值有用：百分比会被
/// 悄悄调松，而一个具体的用例数一旦下降，就必须在提交里显式改掉它并说明是
/// 哪几条退化了。
///
/// 为什么不是 652：见 [`deviations`]，剩下的 8 条是架构决定的偏差，
/// 逐条登记在 `docs/specs/invariants.md` 第 F 节。
///
/// 643 → 644 是 S7 第六刀调的，而且**只可能**是那一刀里 F3 那一半调的：
/// 这条棘轮走 [`render`] → `html.rs`，同一刀把导出换成 comrak 的那一半在
/// `yu-export` 里，动不到这条路上的任何一个字节。两件事因此分在两个 commit。
const RAW_PASS_BASELINE: usize = 644;

struct Example {
    number: usize,
    section: String,
    markdown: String,
    html: String,
}

fn spec_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/commonmark/spec.json")
}

fn load_examples() -> Vec<Example> {
    let path = spec_path();
    let raw =
        std::fs::read(&path).unwrap_or_else(|error| panic!("读不到 {}：{error}", path.display()));

    let digest = sha256(&raw);
    assert_eq!(
        digest, SPEC_SHA256,
        "third_party/commonmark/spec.json 的内容与登记的校验和不符。\
         规范换版必须是一次显式提交，并按该目录 README 的四步走。"
    );

    let parsed: serde_json::Value =
        serde_json::from_slice(&raw).expect("spec.json 应该是合法 JSON");
    let array = parsed.as_array().expect("spec.json 顶层是数组");
    let examples: Vec<Example> = array
        .iter()
        .map(|value| Example {
            number: value["example"].as_u64().expect("example 是数字") as usize,
            section: value["section"].as_str().unwrap_or("").to_owned(),
            markdown: value["markdown"]
                .as_str()
                .expect("markdown 是字符串")
                .to_owned(),
            html: value["html"].as_str().expect("html 是字符串").to_owned(),
        })
        .collect();
    assert_eq!(
        examples.len(),
        EXPECTED_EXAMPLE_COUNT,
        "用例数变了。这不是可以顺手改掉的数字——它是「差分测试真的跑到了」的凭据"
    );
    examples
}

fn render(markdown: &str) -> String {
    let parsed = parse(markdown).expect("规范用例都很短，不会超出长度上限");
    html::render(markdown, parsed.tree())
}

#[test]
fn commonmark_spec_pass_rate() {
    let examples = load_examples();
    let registry = deviations();

    let mut failures: Vec<&Example> = Vec::new();
    let mut unexpected_passes: Vec<usize> = Vec::new();
    let mut raw_passes = 0_usize;

    for example in &examples {
        let matched = render(&example.markdown) == example.html;
        let registered = registry.contains_key(&example.number);
        if matched {
            raw_passes += 1;
            if registered {
                unexpected_passes.push(example.number);
            }
        } else if !registered {
            failures.push(example);
        }
    }

    // 登记表必须是紧的：修好的偏差要从表里删掉。
    assert!(
        unexpected_passes.is_empty(),
        "这些用例已登记为偏差却通过了，说明登记表过期了：{unexpected_passes:?}\n\
         请从 docs/specs/invariants.md 第 F 节与本文件的 deviations() 里删掉它们。"
    );

    if !failures.is_empty() {
        let mut by_section: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for failure in &failures {
            by_section
                .entry(failure.section.as_str())
                .or_default()
                .push(failure.number);
        }
        let mut report = String::new();
        for (section, numbers) in &by_section {
            report.push_str(&format!("  {section}: {numbers:?}\n"));
        }
        let sample = failures[0];
        panic!(
            "{} 条规范用例未通过，且未登记为有意偏差：\n{report}\n\
             第一条（#{}，{}）：\n--- markdown ---\n{}\n--- 期望 ---\n{}\n--- 实际 ---\n{}\n\
             要么修解析器，要么确认这是有意偏差并同时登记进 deviations() 与 \
             docs/specs/invariants.md 第 F 节。不允许只改数字。",
            failures.len(),
            sample.number,
            sample.section,
            sample.markdown,
            sample.html,
            render(&sample.markdown),
        );
    }

    assert!(
        raw_passes >= RAW_PASS_BASELINE,
        "原始通过率退化了：{raw_passes}/{} 低于基线 {RAW_PASS_BASELINE}。\n\
         这个基线只能往上调。",
        examples.len()
    );
    println!(
        "CommonMark 0.31.2：{raw_passes}/{} 逐字节通过（{:.2}%），{} 条已登记偏差",
        examples.len(),
        raw_passes as f64 * 100.0 / examples.len() as f64,
        registry.len()
    );
}

/// GFM 的任务项在 `yu-syntax` 里是无条件开着的，而 CommonMark 不认识它。
///
/// 这两件事目前不打架，理由只有一个：**652 条用例里一条任务项都没有**。
/// 上面那个 643 的棘轮因此完全没有被 GFM 扩展动过。
///
/// 但这是一条运气好的事实，不是一条被保证的性质——规范换版加进一条
/// `- [ ] foo`，棘轮会静静地掉一格，而提交里看到的只是「基线从 643 调到
/// 642」。所以把这条事实钉成断言：将来它不成立时，红的是这条测试，
/// 而不是那个数字。
#[test]
fn tasklist_syntax_is_absent_from_the_spec() {
    let examples = load_examples();
    let hits: Vec<usize> = examples
        .iter()
        .filter(|example| contains_task_marker(&example.markdown))
        .map(|example| example.number)
        .collect();
    assert!(
        hits.is_empty(),
        "规范用例里出现了任务项标记：{hits:?}\n\
         `yu-syntax` 无条件解析 GFM 任务项，这些用例的期望输出是按 CommonMark \
         写的，两者会打架。要么把它们登记进 deviations() 与不变量第 F 节，\
         要么让任务项变成可配置的。不要只调 RAW_PASS_BASELINE。"
    );
}

/// 用例文本里有没有 `[ ]` / `[x]` / `[X]`。
///
/// 判据比解析器宽：解析器还要求它在列表项的第一个内容块开头（见
/// `block.rs::starts_task`）。宽一点是有意的——这条断言要在任务项**可能**
/// 被触发之前就红，而不是在恰好触发时才红。
fn contains_task_marker(markdown: &str) -> bool {
    markdown.as_bytes().windows(3).any(|window| {
        window[0] == b'[' && matches!(window[1], b' ' | b'x' | b'X') && window[2] == b']'
    })
}

/// 已登记的有意偏差：用例号 -> 不变量第 F 节的编号。
///
/// 这张表与 `docs/specs/invariants.md` 第 F 节一一对应。往里加一行之前必须
/// 先确认它是**有意**的：解析器错了不算偏差，算 bug。
fn deviations() -> BTreeMap<usize, &'static str> {
    let mut registry = BTreeMap::new();
    // F1：引用式链接的括号配对不查 reference table。不变量 C6 的直接后果。
    for number in [512, 523, 528, 569, 571] {
        registry.insert(number, "F1");
    }
    // F2：制表符不展开。
    for number in [5, 6, 7] {
        registry.insert(number, "F2");
    }
    // F3 曾经在这里（540：`[ẞ]` 配 `[SS]:`）。S7 第六刀接了 `caseless`，
    // 产品链路与参考渲染一起换成 full case folding，它变绿了，登记随之删掉
    // ——上面 `unexpected_passes` 那道门就是为了逼出这一次删除。
    registry
}

// ---------------------------------------------------------------------------
// 一个足够用的 SHA-256。
//
// 不为一次校验引入依赖：这里只需要「文件没被换掉」这一个判断，实现是
// FIPS 180-4 的标准写法，二十行。
// ---------------------------------------------------------------------------

fn sha256(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    let bit_length = (data.len() as u64) * 8;
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());

    for block in message.chunks_exact(64) {
        let mut w = [0_u32; 64];
        for (index, chunk) in block.chunks_exact(4).enumerate() {
            w[index] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
    state.iter().map(|word| format!("{word:08x}")).collect()
}

/// 逐条打印失败详情，供分诊用。默认不跑。
///
/// `cargo test -p yu-syntax --test commonmark_spec -- --ignored --nocapture spec_report`
/// 环境变量 `SPEC_ONLY` 可以按用例号（逗号分隔）或章节名子串过滤。
#[test]
#[ignore = "诊断用，不是门禁"]
fn spec_report() {
    let filter = std::env::var("SPEC_ONLY").unwrap_or_default();
    let numbers: Vec<usize> = filter
        .split(',')
        .filter_map(|part| part.trim().parse().ok())
        .collect();
    for example in load_examples() {
        let actual = render(&example.markdown);
        if actual == example.html {
            continue;
        }
        let selected = if !numbers.is_empty() {
            numbers.contains(&example.number)
        } else if filter.is_empty() {
            true
        } else {
            example.section.contains(&filter)
        };
        if !selected {
            continue;
        }
        let tree = parse(example.markdown.as_str())
            .expect("短用例")
            .into_tree();
        println!(
            "#{} [{}]\n  md   {:?}\n  want {:?}\n  got  {:?}\n  tree {}",
            example.number,
            example.section,
            example.markdown,
            example.html,
            actual,
            tree.to_sexp()
        );
    }
}
