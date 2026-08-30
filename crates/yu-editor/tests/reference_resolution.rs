//! 候选引用要查表才算数，而表是**文档全局**的。
//!
//! 不变量 C6 规定 parser 只产出候选引用：`[文字][标签]` 在树里是一个 `Link`
//! 节点，无论 `标签` 有没有被定义过。成立与否由装饰阶段判定——于是装饰第一次
//! 依赖了一件**不在这个块里**的事实。
//!
//! 而 `DecorationCache` 是按块留的：range 与 kind 对得上就复用。补一条定义、
//! 删一条定义，用到它的那个块一个字节都没变，缓存照样命中——画面上是一条
//! 写对了却画成普通文字的引用，或者一条已经失效却还画成链接的引用。两种都
//! 不报错。这个文件压的就是那条依赖。

use yu_editor::DecorationCache;
use yu_markdown::parse;
use yu_text::{TextBuffer, TextSnapshot};

const WITHOUT: &str = "[文字][标签]\n";
const WITH: &str = "[文字][标签]\n\n[标签]: /目标\n";

fn snapshot(source: &str) -> TextSnapshot {
    TextBuffer::new(source.to_owned()).snapshot()
}

/// 这个块上被隐藏的字节数。引用成立时定界符会被藏起来，不成立时一个都不藏。
fn hidden_bytes(source: &str) -> u64 {
    let snapshot = snapshot(source);
    let document = parse(&snapshot);
    let block = document.blocks().get(0).expect("至少一个块");
    let mut cache = DecorationCache::default();
    let decorations = cache
        .get_or_build_block(&document, block)
        .expect("装饰产出不该失败");
    decorations
        .set()
        .all()
        .iter()
        .filter(|entry| entry.decoration.hides_source())
        .map(|entry| entry.range.len())
        .sum()
}

#[test]
fn an_unresolved_reference_keeps_its_source_visible() {
    assert_eq!(
        hidden_bytes(WITHOUT),
        0,
        "查不到定义的候选不是链接，整段按源码画"
    );
    assert!(
        hidden_bytes(WITH) > 0,
        "查得到定义的引用是链接，定界符要藏起来"
    );
}

/// 补一条定义之后，用到它的那个块要重新产装饰——哪怕它自己一个字节都没变。
#[test]
fn adding_a_definition_invalidates_the_cached_decorations() {
    let without = parse(&snapshot(WITHOUT));
    let with = parse(&snapshot(WITH));
    let block = without.blocks().get(0).expect("至少一个块");

    let mut cache = DecorationCache::default();
    cache
        .get_or_build_block(&without, block)
        .expect("装饰产出不该失败");
    assert_eq!(cache.stats().entries(), 1);

    // 第一个块的 range 与 kind 一模一样（`[文字][标签]\n` 那一行没动），
    // 只有引用表变了。按块留的判据在这里会命中，所以必须另有一道。
    assert_eq!(
        with.blocks().get(0).expect("至少一个块").range(),
        block.range()
    );
    cache.retain_blocks(&with);
    assert_eq!(
        cache.stats().entries(),
        0,
        "引用表的内容变了，每一条候选引用的判断都可能翻面"
    );
}

/// 只挪动定义、不改内容的编辑不该白清一遍缓存。
///
/// 指纹折的是标签与目标的**内容哈希**，不是它们的位置。折位置的话，任何一次
/// 编辑都会让整篇文档的装饰重算一遍——不报错，只是每敲一个字都慢。
#[test]
fn moving_a_definition_without_changing_it_keeps_the_cache() {
    let before = parse(&snapshot(WITH));
    let after = parse(&snapshot("[文字][标签]\n\n补一段\n\n[标签]: /目标\n"));
    assert_eq!(
        before.reference_definitions().fingerprint(),
        after.reference_definitions().fingerprint(),
        "定义的内容没变，指纹就不该变"
    );
}
