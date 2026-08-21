//! comrak 差分与确定性 fuzz。
//!
//! # 两种差分，不能混为一谈
//!
//! **comrak 差分**比的是 HTML。comrak 是一个 CommonMark 实现，不是 Yu 的
//! 语义真源（不变量 C6：引用链接的成立与否不由 parser 决定）。因此它只在
//! 一个受限语料上跑：生成器**刻意避开**第 F 节已登记的三条偏差。这样一来
//! 「comrak 不一样」就等价于「Yu 错了」，不需要人工分辨——而这正是这一轮
//! 点名的风险：有意偏差和真 bug 混在一起没人分得清。
//!
//! **结构差分**不需要 oracle。它对**任意**输入检查那些无论如何都必须成立的
//! 性质：不 panic、range 合法（C1）、只靠 position 能还原源码（C2）、
//! 增量等于全量（C3）。因为不比 HTML，它可以喂任何东西。
//!
//! # fuzz 怎么进门禁
//!
//! `tools/verify.sh` 是以退出码为准的**确定性**门禁，随机 fuzz 不是。所以
//! 分成两半：
//!
//! - **这里**是确定性的一半：固定种子的生成器 + `tests/corpus/` 里入库的
//!   历史失败样本。每次运行结果完全一样，进 `cargo test`，进 verify.sh。
//! - **`tools/fuzz.sh`** 是随机的一半：随机种子、有时间预算、单独的 CI job。
//!   它找到的每一个失败都要被最小化后写进 `tests/corpus/`，从此由上面那半
//!   永久看守。fuzz 负责**发现**，corpus 负责**不再复发**。

use std::path::{Path, PathBuf};

use yu_core::{ByteOffset, TextRange};
use yu_syntax::{Tree, TreeFragment, parse, parse_with_fragments};
use yu_text::{Edit, TextBuffer, Transaction};

#[path = "support/generator.rs"]
mod generator;
#[path = "support/html.rs"]
mod html;

/// 确定性语料的规模。够大到能覆盖构造组合，够小到 CI 里跑得完。
const DETERMINISTIC_DOCUMENTS: usize = 2_000;

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

/// `tests/corpus/` 里入库的样本：fuzz 找到过的失败，最小化之后放进来。
fn corpus_files() -> Vec<(String, String)> {
    let dir = corpus_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut cases: Vec<(String, String)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "md"))
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(entry.path()).ok()?;
            Some((name, text))
        })
        .collect();
    cases.sort();
    cases
}

// ---------------------------------------------------------------------------
// comrak 差分
// ---------------------------------------------------------------------------

/// 在避开已登记偏差的语料上，Yu 的 HTML 必须与 comrak 逐字节一致。
#[test]
fn html_matches_comrak_on_deviation_free_documents() {
    let mut mismatches = Vec::new();
    for index in 0..DETERMINISTIC_DOCUMENTS {
        let source = generator::deviation_free_document(seed_for(index));
        let ours = render_html(&source);
        let theirs = comrak::markdown_to_html(&source, &comrak_options());
        if ours != theirs {
            mismatches.push((index, source, ours, theirs));
            if mismatches.len() >= 3 {
                break;
            }
        }
    }
    if let Some((index, source, ours, theirs)) = mismatches.first() {
        panic!(
            "第 {index} 份生成文档与 comrak 不一致（共 {} 处）：\n\
             --- markdown ---\n{source}\n--- comrak ---\n{theirs}\n--- yu ---\n{ours}\n\
             这个语料刻意避开了不变量第 F 节的三条已登记偏差，所以差异只可能是 \
             Yu 的 bug，不要往 F 节加条目来消掉它。",
            mismatches.len()
        );
    }
}

/// 入库的历史失败样本也要与 comrak 一致——除非样本文件名以 `structural-`
/// 开头，那种是只验结构性质的（输入可能落在已登记偏差上）。
#[test]
fn corpus_files_match_comrak() {
    for (name, source) in corpus_files() {
        if name.starts_with("structural-") {
            continue;
        }
        let ours = render_html(&source);
        let theirs = comrak::markdown_to_html(&source, &comrak_options());
        assert_eq!(
            ours, theirs,
            "corpus/{name} 与 comrak 不一致\n--- markdown ---\n{source}"
        );
    }
}

// ---------------------------------------------------------------------------
// 结构差分：任意输入
// ---------------------------------------------------------------------------

/// 任意输入下都必须成立的性质。不比 HTML，所以不需要 oracle，也就不受
/// 已登记偏差影响。
#[test]
fn structural_invariants_hold_on_arbitrary_documents() {
    for index in 0..DETERMINISTIC_DOCUMENTS {
        let source = generator::arbitrary_document(seed_for(index));
        check_structural_invariants(&source)
            .unwrap_or_else(|reason| panic!("第 {index} 份任意文档：{reason}\n源码 {source:?}"));
    }
    for (name, source) in corpus_files() {
        check_structural_invariants(&source)
            .unwrap_or_else(|reason| panic!("corpus/{name}：{reason}\n源码 {source:?}"));
    }
}

/// 任意输入下必须成立的四条。确定性测试与 `fuzz_soak` 共用同一个检查体
/// ——fuzz 与门禁检查的是同一件事，区别只在种子。
///
/// 返回 `Err` 而不是 panic，好让 fuzz 驱动能把失败样本写进 corpus 再退出。
fn check_structural_invariants(source: &str) -> Result<(), String> {
    let parsed = parse(source).map_err(|error| format!("全量解析失败：{error}"))?;
    let tree = parsed.tree();

    // C1：range 有序、有效、不越界、不落在字符中间。
    check_ranges(tree, 0, source)?;

    // C2：只靠 position 能字节级还原源码。
    let rebuilt = rebuild(tree, 0, source);
    if rebuilt != source {
        return Err(format!("C2 失败：还原出 {rebuilt:?}"));
    }

    // C3：在每个字符边界上插一个字符，增量必须等于全量。
    //
    // 只取有限个位置：任意文档可能很长，而 bug 不挑位置——固定步长足以在
    // 大量样本上覆盖到各种边界。
    let boundaries: Vec<usize> = source
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(source.len()))
        .collect();
    let step = boundaries.len().div_ceil(16).max(1);
    for offset in boundaries.into_iter().step_by(step) {
        for insert in ["x", "\n", "`", ">"] {
            check_incremental_step(source, offset, insert)?;
        }
    }
    Ok(())
}

fn check_incremental_step(source: &str, offset: usize, insert: &str) -> Result<(), String> {
    let mut buffer = TextBuffer::new(source.to_owned());
    let fragments = {
        let parsed = parse(&buffer.snapshot()).map_err(|error| error.to_string())?;
        TreeFragment::from_tree(parsed.tree())
    };
    let range = TextRange::new(
        ByteOffset::try_from(offset).map_err(|_| "偏移溢出".to_owned())?,
        ByteOffset::try_from(offset).map_err(|_| "偏移溢出".to_owned())?,
    )
    .ok_or_else(|| "空 range 应当合法".to_owned())?;
    let transaction = Transaction::new(buffer.revision(), [Edit::new(range, insert)]);
    let applied = buffer
        .apply(&transaction)
        .map_err(|error| format!("{error:?}"))?;
    let snapshot = applied.result_snapshot();
    let moved = TreeFragment::apply_change_set(&fragments, applied.change_set());
    let incremental = parse_with_fragments(snapshot, &moved)
        .map_err(|error| error.to_string())?
        .into_tree();
    let full = parse(snapshot.as_str())
        .map_err(|error| error.to_string())?
        .into_tree();
    if incremental != full {
        return Err(format!(
            "C3 失败：在 {offset} 处插入 {insert:?} 之后\n增量 {}\n全量 {}",
            incremental.to_sexp(),
            full.to_sexp()
        ));
    }
    Ok(())
}

fn check_ranges(tree: &Tree, from: u32, source: &str) -> Result<(), String> {
    let to = from + tree.len_bytes();
    let len = u32::try_from(source.len()).unwrap_or(u32::MAX);
    if to > len {
        return Err(format!(
            "{} 的终点 {to} 超出源码长度 {len}",
            tree.kind().name()
        ));
    }
    if !source.is_char_boundary(from as usize) || !source.is_char_boundary(to as usize) {
        return Err(format!(
            "{} 的 range {from}..{to} 落在 UTF-8 字符中间",
            tree.kind().name()
        ));
    }
    let mut previous_end = from;
    for index in 0..tree.child_count() {
        let (child, position) = tree.child(index).ok_or("子节点下标越界")?;
        let child_from = from + position;
        if child_from < previous_end {
            return Err(format!("{} 的子节点互相交叉", tree.kind().name()));
        }
        if child_from + child.len_bytes() > to {
            return Err(format!("{} 的子节点超出父节点范围", tree.kind().name()));
        }
        previous_end = child_from + child.len_bytes();
        check_ranges(child, child_from, source)?;
    }
    Ok(())
}

fn rebuild(tree: &Tree, from: u32, source: &str) -> String {
    let to = from + tree.len_bytes();
    if tree.child_count() == 0 {
        return source[from as usize..to as usize].to_owned();
    }
    let mut out = String::new();
    let mut cursor = from;
    for index in 0..tree.child_count() {
        let Some((child, position)) = tree.child(index) else {
            break;
        };
        let child_from = from + position;
        if child_from > cursor {
            out.push_str(&source[cursor as usize..child_from as usize]);
        }
        out.push_str(&rebuild(child, child_from, source));
        cursor = (child_from + child.len_bytes()).max(cursor);
    }
    if cursor < to {
        out.push_str(&source[cursor as usize..to as usize]);
    }
    out
}

/// comrak 的默认选项会把原始 HTML 换成 `<!-- raw HTML omitted -->`
/// （`render.unsafe_ = false`），那是它作为 Web 渲染器的安全默认值，
/// 不是 CommonMark 的行为。规范要求原样输出，所以这里必须打开。
fn comrak_options() -> comrak::Options<'static> {
    let mut options = comrak::Options::default();
    options.render.r#unsafe = true;
    options
}

fn render_html(source: &str) -> String {
    let parsed = parse(source).expect("生成的文档不会超长");
    html::render(source, parsed.tree())
}

/// 每份文档一个独立种子，改动 `DETERMINISTIC_DOCUMENTS` 不会让已有文档变样。
fn seed_for(index: usize) -> u64 {
    0x5955_5F53_594E_5441_u64 ^ (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// corpus 目录必须存在且有 README，否则「fuzz 找到的样本要入库」这条约定
/// 会随着目录被误删而无声消失。
#[test]
fn corpus_directory_is_documented() {
    let readme = corpus_dir().join("README.md");
    assert!(
        Path::new(&readme).is_file(),
        "{} 不存在。corpus 是 fuzz 与确定性门禁之间的交接点，\
         没有它 fuzz 找到的问题不会被永久看守。",
        readme.display()
    );
}

// ---------------------------------------------------------------------------
// 随机 fuzz 的驱动
//
// 默认不跑。`tools/fuzz.sh` 通过环境变量启动它，CI 里是一个独立 job。
// 它与上面那些确定性测试共用同一套检查体——fuzz 与门禁检查的是同一件事，
// 区别只在种子是随机的、有时间预算。
// ---------------------------------------------------------------------------

/// 随机 fuzz。找到失败就把输入写进 `tests/corpus/` 并让测试红。
///
/// ```text
/// tools/fuzz.sh 120
/// ```
#[test]
#[ignore = "随机、有时间预算，不是确定性门禁；由 tools/fuzz.sh 启动"]
fn fuzz_soak() {
    let seconds: u64 = std::env::var("YU_FUZZ_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(30);
    let base_seed: u64 = std::env::var("YU_FUZZ_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos() as u64)
                .unwrap_or(0x5955)
        });

    println!("fuzz 起始种子 {base_seed}，预算 {seconds} 秒");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let mut iterations = 0_u64;

    while std::time::Instant::now() < deadline {
        let seed = base_seed.wrapping_add(iterations.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        iterations += 1;

        // 一半任意文档（只验结构），一半回避偏差的文档（另比 comrak）。
        let arbitrary = iterations.is_multiple_of(2);
        let source = if arbitrary {
            generator::arbitrary_document(seed)
        } else {
            generator::deviation_free_document(seed)
        };

        if let Err(reason) = check_structural_invariants(&source) {
            report_failure(seed, &source, &reason, true);
        }
        if !arbitrary {
            let ours = render_html(&source);
            let theirs = comrak::markdown_to_html(&source, &comrak_options());
            if ours != theirs {
                report_failure(
                    seed,
                    &source,
                    &format!("与 comrak 不一致\n--- comrak ---\n{theirs}\n--- yu ---\n{ours}"),
                    false,
                );
            }
        }
    }
    println!("fuzz 跑完 {iterations} 份文档，没有发现问题");
}

/// 把失败样本写进 corpus 并 panic。
///
/// 写文件是为了让「fuzz 发现 → corpus 看守」这一步不依赖人去复制粘贴。
/// 但它只是原始样本，还需要人工最小化与改名，见 `tests/corpus/README.md`。
fn report_failure(seed: u64, source: &str, reason: &str, structural: bool) -> ! {
    let prefix = if structural { "structural-" } else { "" };
    let path = corpus_dir().join(format!("{prefix}fuzz-seed-{seed}.md"));
    let written = std::fs::write(&path, source).is_ok();
    panic!(
        "fuzz 发现失败（种子 {seed}）：{reason}\n--- markdown ---\n{source:?}\n\
         样本{}写入 {}\n\
         下一步：最小化、按 tests/corpus/README.md 改名、确认它在修复前确实变红。",
        if written { "已" } else { "未能" },
        path.display()
    );
}
