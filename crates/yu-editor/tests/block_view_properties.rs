//! `BlockView` 的性质。
//!
//! # 这个文件顶替了什么
//!
//! S5 全程靠一条几何差分守着：新引擎与 v1 的 `LayoutSnapshot` 喂同一个
//! `Projection`，逐点比对行盒、簇、图片盒、表格网格、caret 与 hit-test。
//! 它按「有没有发生软换行」分两个口径，因为 v1 没有 UAX #14，断点必然不同。
//!
//! **`LayoutSnapshot` 删掉之后那个 oracle 就没有了。** 剩下的保障是两样：
//! 这里的性质测试，以及真实窗口——`docs/specs/manual-acceptance-macos.md`。
//! 性质测试压得住的是自洽（caret 与 hit-test 说的是同一件事、簇铺满视觉
//! 文本、源码区间不越界）；压不住的是「画出来好不好看」，那件事从来只能
//! 靠真实窗口。
//!
//! 差分历史留在 git 里：`git log --oneline -- crates/yu-editor/tests/geometry_differential.rs`。

use yu_core::{ByteOffset, TextRange, VisualOffset};
use yu_decoration::Bias;
use yu_editor::{
    BlockDecorations, BlockLayoutInput, BlockView, LayoutConfig, LayoutPoint, MonospaceMetrics,
    VisualText,
};
use yu_markdown::ExtensionSet;
use yu_syntax::parse as parse_syntax;
use yu_text::{TextBuffer, TextSnapshot};

/// 覆盖每一种块：段落、标题、引用、列表、任务、表格、代码围栏、
/// 引用定义、图片。
const CORPUS: &[&str] = &[
    "",
    "plain paragraph\n",
    "# h1 title\n",
    "### h3 *em* title\n",
    "> quoted text\n",
    "> > nested quote\n",
    "- item one\n",
    "  - indented item\n",
    "1. ordered item\n",
    "- [ ] task item\n",
    "- [x] done item\n",
    "![alt text](/img.png)\n",
    "before ![alt](/i.png) after\n",
    "| a | b |\n|---|---|\n| 1 | 2 |\n",
    "| long header | x |\n| --- | --- |\n| c | d |\n| e | f |\n",
    "```rust\nlet x = 1;\n```\n",
    "[ref]: /url\n",
    "text with `code` and **strong**\n",
    "中文 *强调* 混排\n",
    "one two three four five six seven eight nine\n",
];

const WIDTHS: &[f32] = &[12.0, 40.0, 160.0];

/// 第 `index` 个块的装饰与它的视觉文本。
fn decorate(snapshot: &TextSnapshot, index: usize) -> Option<(BlockDecorations, VisualText)> {
    let markdown = yu_markdown::parse(snapshot);
    let block = markdown.blocks().get(index)?;
    let tree = parse_syntax(snapshot).expect("测试文档很短").into_tree();
    let decorations = ExtensionSet::markdown()
        .decorate(snapshot, &tree, block, None)
        .expect("装饰产出");
    let visual = VisualText::new(snapshot, decorations.range(), decorations.set().clone())
        .expect("视觉文本");
    Some((decorations, visual))
}

fn view(source: &str, width: f32) -> Option<BlockView> {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let (decorations, visual) = decorate(&snapshot, 0)?;
    let config = LayoutConfig::new(width, 10.0).with_default_advance(2.0);
    Some(
        BlockView::build(
            &visual,
            &decorations,
            config,
            &MonospaceMetrics::new(config.default_advance()),
        )
        .expect("BlockView"),
    )
}

/// 派生出来的视觉文本必须与装饰投影说的一样长，样式区间必须无缝铺满它。
///
/// 漏掉半段会画出少了几个字的一行，既不 panic 也不报错。
#[test]
fn the_derived_input_tiles_the_visual_text() {
    let config = LayoutConfig::new(160.0, 10.0).with_default_advance(2.0);
    let metrics = MonospaceMetrics::new(config.default_advance());
    for source in CORPUS {
        let buffer = TextBuffer::new((*source).to_owned());
        let snapshot = buffer.snapshot();
        let Some((decorations, visual)) = decorate(&snapshot, 0) else {
            continue;
        };
        let input = BlockLayoutInput::from_decorations(&decorations, &visual, config, &metrics)
            .expect("派生输入");
        assert_eq!(
            VisualOffset::try_from(input.text().len()).expect("短"),
            visual.visual_len(),
            "语料 {source:?} 的视觉长度"
        );
        let mut cursor = VisualOffset::ZERO;
        for run in input.layout_input().runs() {
            assert_eq!(
                run.visual().start(),
                cursor,
                "语料 {source:?} 的 run 没铺满"
            );
            cursor = run.visual().end();
        }
        assert_eq!(cursor, visual.visual_len());
    }
}

/// 簇按视觉顺序首尾相接铺满整块，源码区间不越出块边界。
#[test]
fn clusters_tile_the_visual_text_and_stay_inside_the_block() {
    for source in CORPUS {
        for width in WIDTHS {
            let Some(view) = view(source, *width) else {
                continue;
            };
            let at = format!("语料 {source:?} 宽度 {width}");
            let block = view.source_range();
            let mut cursor = VisualOffset::ZERO;
            for cluster in view.clusters() {
                assert_eq!(cluster.visual().start(), cursor, "{at} 的簇没铺满");
                cursor = cluster.visual().end();
                assert!(
                    cluster.source().start() >= block.start()
                        && cluster.source().end() <= block.end(),
                    "{at} 的簇源码越界"
                );
            }
            assert_eq!(cursor, view.visual_len(), "{at} 的簇没铺到末尾");
        }
    }
}

/// 每条行都覆盖一段源码，首尾相接铺满整块。
///
/// 代码围栏的收尾那一行看起来是空的，但它拥有 ``` 那几个字节。按行查源码
/// 漏掉它们不会报错。
#[test]
fn lines_tile_the_block_source() {
    for source in CORPUS {
        for width in WIDTHS {
            let Some(view) = view(source, *width) else {
                continue;
            };
            let at = format!("语料 {source:?} 宽度 {width}");
            let block = view.source_range();
            let mut cursor = block.start();
            for line in view.lines() {
                if view.table().is_some() {
                    // 表格的「行」是网格的行。分隔行（`|---|`）是 parser 拥有
                    // 的一整行源码，没有可见的格，因此不属于任何一行——
                    // 网格行之间本来就有缝。
                    assert!(line.source().start() >= cursor, "{at} 的行没有升序");
                    assert!(line.source().end() <= block.end(), "{at} 的行越界");
                } else {
                    assert_eq!(line.source().start(), cursor, "{at} 第 {} 行", line.index());
                }
                cursor = line.source().end();
            }
            if view.table().is_none() {
                assert_eq!(cursor, block.end(), "{at} 的行没铺到块尾");
            }
        }
    }
}

/// caret 与 hit-test 说的是同一件事。
///
/// 两处各写一遍规则，就会「点一下，光标跳到别处」——不 panic，不报错，
/// 只是不听话。S5 在 bidi 那一刀抓到过一次同样的毛病。
#[test]
fn hit_test_lands_on_a_caret_position() {
    for source in CORPUS {
        for width in WIDTHS {
            let Some(view) = view(source, *width) else {
                continue;
            };
            let at = format!("语料 {source:?} 宽度 {width}");
            for line in view.lines() {
                let y = line.y() + line.height() * 0.5;
                let mut xs = vec![0.0_f32, line.width(), line.width() + 3.0];
                for index in line.cluster_range() {
                    let cluster = view.clusters()[index];
                    xs.push(cluster.x());
                    xs.push(cluster.x() + cluster.width() * 0.25);
                    xs.push(cluster.x() + cluster.width() * 0.75);
                    xs.push(cluster.x() + cluster.width());
                }
                for x in xs {
                    let point = LayoutPoint::new(x, y);
                    let hit = view
                        .hit_test(point)
                        .unwrap_or_else(|error| panic!("{at} 的 hit: {error}"));
                    if hit.image().is_some() {
                        continue;
                    }
                    let caret = view
                        .caret_for_visual(hit.visual(), hit.bias())
                        .unwrap_or_else(|error| panic!("{at} 的 caret: {error}"));
                    assert_eq!(
                        caret.point(),
                        hit.point(),
                        "{at} x={x} 的 hit 与 caret 对不上"
                    );
                }
            }
        }
    }
}

/// 每个源码边界都能问出一个 caret，而且它落在这一块里。
#[test]
fn every_source_boundary_has_a_caret_inside_the_block() {
    for source in CORPUS {
        for width in WIDTHS {
            let Some(view) = view(source, *width) else {
                continue;
            };
            let at = format!("语料 {source:?} 宽度 {width}");
            let block = view.source_range();
            let mut offsets = vec![block.start(), block.end()];
            for cluster in view.clusters() {
                offsets.push(cluster.source().start());
                offsets.push(cluster.source().end());
            }
            offsets.sort_by_key(|offset| offset.get());
            offsets.dedup();
            for offset in offsets {
                for bias in [Bias::Before, Bias::After] {
                    let caret = view
                        .caret_for_source(offset, bias)
                        .unwrap_or_else(|error| panic!("{at} 源码 {}: {error}", offset.get()));
                    assert!(caret.line() < view.lines().len(), "{at} 的行号越界");
                    assert!(caret.point().x() >= 0.0, "{at} 的 caret 在块外");
                    assert!(caret.point().y() >= 0.0, "{at} 的 caret 在块上方");
                    assert!(
                        caret.point().y() < view.height().max(1.0),
                        "{at} 的 caret 在块下方"
                    );
                }
            }
        }
    }
}

/// 块高盖得住每一条行与每一张图片。
///
/// 盖不住就是「可滚动范围来自另一套几何」——长文档尾部滚不到，不报错。
#[test]
fn the_block_height_covers_every_line_and_image() {
    for source in CORPUS {
        for width in WIDTHS {
            let Some(view) = view(source, *width) else {
                continue;
            };
            let at = format!("语料 {source:?} 宽度 {width}");
            let height = view.height();
            for line in view.lines() {
                assert!(
                    line.y() + line.height() <= height + f32::EPSILON,
                    "{at} 第 {} 行超出块高",
                    line.index()
                );
            }
            for image in view.images() {
                assert!(
                    image.bounds().y() + image.bounds().height() <= height + f32::EPSILON,
                    "{at} 的图片超出块高"
                );
            }
        }
    }
}

/// 表格块的每一个簇都落在某个单元格里。
///
/// 落不进去的簇会被画在网格外面——那正是 v1 用一句断言拦下来的东西，
/// 搬过来时一并保住。
#[test]
fn every_table_cluster_belongs_to_a_cell() {
    let mut tables = 0_usize;
    for source in CORPUS {
        for width in WIDTHS {
            let Some(view) = view(source, *width) else {
                continue;
            };
            let Some(table) = view.table() else {
                continue;
            };
            tables += 1;
            let at = format!("语料 {source:?} 宽度 {width}");
            for cluster in view.clusters() {
                if cluster.is_line_break() {
                    continue;
                }
                let cell = table.cells().iter().find(|cell| {
                    cell.source().start() <= cluster.source().start()
                        && cluster.source().end() <= cell.source().end()
                });
                let cell = cell.unwrap_or_else(|| panic!("{at} 有一个簇不属于任何单元格"));
                assert_eq!(cluster.line(), cell.row(), "{at} 的簇行号与单元格不一致");
            }
        }
    }
    assert_eq!(tables, 6, "表格语料的数量变了");
}

/// 一次块外的编辑之后，这个块的几何原样保留，源码区间整体平移。
#[test]
fn a_prefix_edit_shifts_the_source_ranges_and_keeps_the_geometry() {
    let source = "first\n\nsecond paragraph\n";
    let mut buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let (decorations, visual) = decorate(&snapshot, 2).expect("第三个块");
    let config = LayoutConfig::new(160.0, 10.0).with_default_advance(2.0);
    let view = BlockView::build(
        &visual,
        &decorations,
        config,
        &MonospaceMetrics::new(config.default_advance()),
    )
    .expect("BlockView");

    let transaction = yu_text::Transaction::new(
        snapshot.revision(),
        [yu_text::Edit::new(TextRange::empty(ByteOffset::ZERO), "xx")],
    );
    let applied = buffer.apply(&transaction).expect("插入");
    let after = applied.result_snapshot().clone();
    let mapped = view
        .map_through(applied.change_set(), &after)
        .expect("重映射")
        .expect("块仍然存在");

    assert_eq!(mapped.lines().len(), view.lines().len());
    for (before, after) in view.lines().iter().zip(mapped.lines()) {
        assert_eq!(before.bounds(), after.bounds(), "行盒不该动");
        assert_eq!(
            before.source().start().get() + 2,
            after.source().start().get(),
            "行的源码该整体后移"
        );
    }
    for (before, after) in view.clusters().iter().zip(mapped.clusters()) {
        assert_eq!(before.x(), after.x(), "簇的 x 不该动");
        assert_eq!(
            before.source().start().get() + 2,
            after.source().start().get(),
            "簇的源码该整体后移"
        );
    }
}
