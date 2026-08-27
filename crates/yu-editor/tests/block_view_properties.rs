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
    "مرحبا بالعالم\n",
    "abc مرحبا def\n",
    "שלום *hello* עולם\n",
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

/// 一行里的簇是不是按 x 递增排的。
///
/// 文字流不保证（bidi 会重排），表格网格本该保证——列不重叠的时候。
fn x_ascending(view: &BlockView, clusters: std::ops::Range<usize>) -> bool {
    let mut previous = f32::NEG_INFINITY;
    for index in clusters {
        let cluster = view.clusters()[index];
        if cluster.is_line_break() {
            continue;
        }
        if cluster.x() < previous {
            return false;
        }
        previous = cluster.x();
    }
    true
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

/// **点一下，光标停在离手指最近的那个 caret 上。**
///
/// 这条曾经写成「`hit_test` 给的点等于 `caret_for_visual(hit.visual, hit.bias)`
/// 给的点」——那是**自证的**：`hit_test` 的返回值本来就是拿后者算出来的，
/// 比的是同一次计算的两遍读法。它全绿的时候 `hit_test` 在 bidi 行里最远能
/// 差十个像素。
///
/// 真正的判据必须来自 `hit_test` **之外**：这一行上所有够得着的 caret 位置
/// 里，没有哪个比它给的那个更靠近点击处。落差不 panic、不报错，只是点一下
/// 光标跳到别处。
#[test]
fn hit_test_lands_on_the_nearest_caret() {
    for source in CORPUS {
        for width in WIDTHS {
            let Some(view) = view(source, *width) else {
                continue;
            };
            // 表格走的是另一条路（网格，不是文字流），而它现在过不了这一
            // 条——登记在 overview 第 8 节 S6 的「还没做的」里。表格的命中
            // 测试由下面那条弱一些的性质守着。
            if view.table().is_some() {
                continue;
            }
            let at = format!("语料 {source:?} 宽度 {width}");
            for line in view.lines() {
                let y = line.y() + line.height() * 0.5;
                // 这一行上够得着的 caret 位置。取簇的两端而不是 `hit_test`
                // 自己的输出——判据不能来自被测的那条路。
                let mut carets: Vec<f32> = Vec::new();
                for index in line.cluster_range() {
                    let cluster = view.clusters()[index];
                    for (visual, bias) in [
                        (cluster.visual().start(), Bias::After),
                        (cluster.visual().end(), Bias::Before),
                    ] {
                        if let Ok(caret) = view.caret_for_visual(visual, bias)
                            && caret.line() == line.index()
                        {
                            carets.push(caret.point().x());
                        }
                    }
                }
                if carets.is_empty() {
                    continue;
                }

                let mut probes = carets.clone();
                probes.push(0.0);
                probes.push(line.width());
                probes.push(line.width() + 3.0);
                for x in probes {
                    let hit = view
                        .hit_test(LayoutPoint::new(x, y))
                        .unwrap_or_else(|error| panic!("{at} 的 hit: {error}"));
                    if hit.image().is_some() {
                        continue;
                    }
                    let got = (hit.point().x() - x).abs();
                    let nearest = carets
                        .iter()
                        .map(|caret| (caret - x).abs())
                        .fold(f32::INFINITY, f32::min);
                    assert!(
                        got <= nearest + 0.001,
                        "{at} 行 {} x={x}：光标停在 {}（差 {got}），\
                         而最近的 caret 只差 {nearest}",
                        line.index(),
                        hit.point().x()
                    );
                }
            }
        }
    }
}

/// 表格里点一下，光标停在**点到的那一行**上，而且落在那一行的视觉区间里。
///
/// 比不上上面那条「最近的 caret」——表格现在过不了它，理由见上面的注释与
/// overview 的登记。这一条压的是不会跑到别的行去、不会给出这一行以外的
/// 偏移；那两样错了就是「点第三行选中了第一行」。
#[test]
fn a_table_hit_stays_on_the_row_it_landed_in() {
    for source in CORPUS {
        for width in WIDTHS {
            let Some(view) = view(source, *width) else {
                continue;
            };
            if view.table().is_none() {
                continue;
            }
            let at = format!("语料 {source:?} 宽度 {width}");
            for line in view.lines() {
                let y = line.y() + line.height() * 0.5;
                for x in [
                    0.0_f32,
                    line.width() * 0.5,
                    line.width(),
                    line.width() + 3.0,
                ] {
                    let hit = view
                        .hit_test(LayoutPoint::new(x, y))
                        .unwrap_or_else(|error| panic!("{at} 的 hit: {error}"));
                    if hit.image().is_some() {
                        continue;
                    }
                    assert_eq!(
                        hit.line(),
                        line.index(),
                        "{at} 在第 {} 行 x={x} 点击，光标跑到了第 {} 行",
                        line.index(),
                        hit.line()
                    );
                    assert!(
                        hit.visual() >= line.visual().start()
                            && hit.visual() <= line.visual().end(),
                        "{at} 第 {} 行 x={x} 的偏移 {:?} 不在这一行的视觉区间 {:?} 里",
                        line.index(),
                        hit.visual(),
                        line.visual()
                    );
                }
            }
        }
    }
}

/// 点在一个字的左半边，光标停在它**前面**。
///
/// 表格那一路仍然是「过了中点算下一个」的按 x 扫描——这一条把那句话钉住。
/// 只断左半边：右半边落在单元格的最后一个字上时会跨到下一格去，那是表格几何
/// 的事，见上面登记的那一条。
#[test]
fn a_click_on_the_left_half_of_a_glyph_lands_before_it() {
    for source in CORPUS {
        for width in WIDTHS {
            let Some(view) = view(source, *width) else {
                continue;
            };
            if view.table().is_none() {
                continue;
            }
            let at = format!("语料 {source:?} 宽度 {width}");
            for line in view.lines() {
                // 按 x 扫描的前提是一行里的簇 x 递增。窄到放不下的表格里列会
                // 重叠，前提不成立——那一条登记在 overview 第 8 节 S6 的
                // 「还没做的」里，随 widget 化一起解决。
                if !x_ascending(&view, line.cluster_range()) {
                    continue;
                }
                let y = line.y() + line.height() * 0.5;
                for index in line.cluster_range() {
                    let cluster = view.clusters()[index];
                    if cluster.is_line_break() || cluster.width() <= 0.0 {
                        continue;
                    }
                    let x = cluster.x() + cluster.width() * 0.25;
                    let hit = view
                        .hit_test(LayoutPoint::new(x, y))
                        .unwrap_or_else(|error| panic!("{at} 的 hit: {error}"));
                    if hit.image().is_some() {
                        continue;
                    }
                    assert_eq!(
                        hit.visual(),
                        cluster.visual().start(),
                        "{at} 点在簇 {index}（x={}，宽 {}）的左四分之一处，\
                         光标却停在 {:?}",
                        cluster.x(),
                        cluster.width(),
                        hit.visual()
                    );
                }
            }
        }
    }
}

/// 点在图片盒子里，命中的是那张图。
///
/// 图片是一个 widget，盒子在行里占宽度，而它覆盖的 source 一个字节都不进
/// 视觉文本。少了这一条优先级，点图片会变成把光标放到盒子的某一沿上——
/// 不报错，只是点不动图。
#[test]
fn a_click_inside_an_image_hits_the_image() {
    for source in ["![alt text](/img.png)\n", "before ![alt](/i.png) after\n"] {
        for width in WIDTHS {
            let Some(view) = view(source, *width) else {
                continue;
            };
            let at = format!("语料 {source:?} 宽度 {width}");
            assert!(!view.images().is_empty(), "{at} 应该有图片盒子");
            for image in view.images() {
                let bounds = image.bounds();
                let point = LayoutPoint::new(
                    bounds.x() + bounds.width() * 0.5,
                    bounds.y() + bounds.height() * 0.5,
                );
                let hit = view
                    .hit_test(point)
                    .unwrap_or_else(|error| panic!("{at} 的 hit: {error}"));
                assert!(
                    hit.image().is_some(),
                    "{at} 点在图片盒子正中，命中的却不是图片"
                );
            }
        }
    }
}

/// 整块都被藏起来时，点它落在**块首**。
///
/// 空的围栏代码块（```` ```\n``` ````）视觉上什么都没有：两条围栏都藏了。
/// 落在块首意味着光标还在这个块里；落在块尾就是下一个块的开头，点这个块会
/// 选中下一个——不 panic、不报错，只是点不进去。
///
/// 这一条钉的是**现状**：块首不见得是最理想的答案（真正「在代码块里面」是
/// 两条围栏之间），但块尾一定更差。改它是另一件事。
#[test]
fn a_fully_hidden_block_is_hit_at_its_start() {
    let source = "```\n```\n";
    for width in WIDTHS {
        let Some(view) = view(source, *width) else {
            continue;
        };
        let block = view.source_range();
        for x in [0.0_f32, 4.0, 40.0] {
            let hit = view
                .hit_test(LayoutPoint::new(x, 1.0))
                .expect("空围栏块的 hit");
            assert_eq!(
                hit.source(),
                block.start(),
                "宽度 {width} x={x}：点空围栏块落在了 {}，而块是 {}..{}",
                hit.source().get(),
                block.start().get(),
                block.end().get()
            );
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
/// 图片那一条现在是**导出**的：盒子在行里排，行高把它算进去了。它留着是
/// 因为表格里的图片仍然是排完之后另摆的（`place_images_in_table`）。
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

// -------------------------------------------------------------- 图片 widget

/// 资源没就绪时给一个 placeholder 盒子，排版照常完成（不变量 D7）。
///
/// 「照常完成」是这条不变量的全部内容：解码是异步的，排版不能等它。欠着谁
/// 由 `pending_widgets` 报出来。
#[test]
fn an_unresolved_image_gets_a_placeholder_box() {
    let (view, _) = image_view(&[]);
    assert_eq!(view.images().len(), 1);
    assert_eq!(
        view.layout().pending_widgets().len(),
        1,
        "还没解码的图片必须报在 pending 里"
    );
    let bounds = view.images()[0].bounds();
    assert_eq!(bounds.height(), LINE_HEIGHT);
    assert_eq!(bounds.width(), LINE_HEIGHT * 4.0);
}

/// 解码后的尺寸到位：盒子按长宽比缩进可用宽度，行跟着长高。
///
/// 行不跟着长高的话，图片会压在下一行上——此前正是这样，块高要在
/// `BlockView::height` 里另取一次 max 才盖得住。
#[test]
fn a_ready_image_keeps_its_ratio_and_lifts_its_line() {
    let (placeholder, _) = image_view(&[]);
    let (view, source) = image_view(&[]);
    let ready = image_view(&[(
        source,
        yu_editor::ImageIntrinsicSize::new(200, 100).expect("固有尺寸"),
    )])
    .0;
    assert!(view.layout().pending_widgets().len() == 1);
    assert!(ready.layout().pending_widgets().is_empty());

    let bounds = ready.images()[0].bounds();
    // 可用宽度 40，固有 200×100 → 缩到 40×20。
    assert_eq!(bounds.width(), 40.0);
    assert_eq!(bounds.height(), 20.0);
    assert!(
        ready.lines()[0].height() >= bounds.height(),
        "图片那一行必须容得下它"
    );
    assert!(
        ready.height() > placeholder.height(),
        "块高跟着行走，不必另取一次 max"
    );
}

/// 光标进到图片里，widget 让位，源码原样排出来可编辑（不变量 D7 的回退）。
#[test]
fn a_focused_image_lays_out_its_source_instead_of_a_box() {
    let buffer = TextBuffer::new(IMAGE_SOURCE.to_owned());
    let snapshot = buffer.snapshot();
    let markdown = yu_markdown::parse(&snapshot);
    let block = markdown.blocks().get(0).expect("一个块");
    let tree = parse_syntax(&snapshot).expect("短文档").into_tree();
    let active = TextRange::new(ByteOffset::new(5), ByteOffset::new(5)).expect("升序");
    let decorations = ExtensionSet::markdown()
        .decorate(&snapshot, &tree, block, Some(active))
        .expect("装饰");
    let visual = VisualText::new(&snapshot, decorations.range(), decorations.set().clone())
        .expect("视觉文本");
    let view = BlockView::build(
        &visual,
        &decorations,
        LayoutConfig::new(40.0, LINE_HEIGHT),
        &MonospaceMetrics::new(1.0),
    )
    .expect("排版");
    assert!(view.images().is_empty(), "露出源码时不排盒子");
    assert!(
        view.visual().text().starts_with("![alt]"),
        "整段源码进视觉文本，实际是 {:?}",
        view.visual().text()
    );
}

const IMAGE_SOURCE: &str = "![alt](/img.png)\n";
const LINE_HEIGHT: f32 = 10.0;

/// 一张图自成一段，按给定的固有尺寸表排一遍。返回它与那张图的源码区间。
fn image_view(sizes: &[yu_editor::ImageSize]) -> (BlockView, TextRange) {
    let buffer = TextBuffer::new(IMAGE_SOURCE.to_owned());
    let snapshot = buffer.snapshot();
    let (decorations, visual) = decorate(&snapshot, 0).expect("一个块");
    let yu_editor::BlockWidget::Image(image) = decorations
        .widgets()
        .first()
        .copied()
        .expect("这个块上有一张图");
    let view = BlockView::build_with_images(
        &visual,
        &decorations,
        LayoutConfig::new(40.0, LINE_HEIGHT),
        &MonospaceMetrics::new(1.0),
        sizes,
    )
    .expect("排版");
    (view, image.source())
}

/// 单元格里的图片要把列撑开。
///
/// widget 在视觉字节流里不占位，按样式段切出来的单元格内容一个字节都切不到
/// 它。不算的话，一格里只有一张图的那一列会被压成一条缝，而图片照样按自己
/// 的宽度画出去，压在下一列上——不报错，只是画错。
#[test]
fn an_image_in_a_cell_widens_its_column() {
    let with_image = table_column_widths("| a | ![i](/x.png) |\n| --- | --- |\n| 1 | 2 |\n");
    let without = table_column_widths("| a |  |\n| --- | --- |\n| 1 | 2 |\n");
    assert!(
        with_image[1] > without[1],
        "有图那一列要更宽：{with_image:?} vs {without:?}"
    );
}

fn table_column_widths(source: &str) -> Vec<f32> {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let (decorations, visual) = decorate(&snapshot, 0).expect("一个块");
    let view = BlockView::build(
        &visual,
        &decorations,
        LayoutConfig::new(400.0, LINE_HEIGHT),
        &MonospaceMetrics::new(1.0),
    )
    .expect("排版");
    view.table()
        .expect("这个块是一张表")
        .column_widths()
        .to_vec()
}

/// preedit 之后的 widget 锚点跟着往后挪。
///
/// 样式段与 widget 锚点都排在 canonical 视觉空间里，再由 preedit 整体让位。
/// widget 忘了让位的话，图片会画在 preedit 里那几个字的中间——不报错。
#[test]
fn a_preedit_pushes_a_later_image_anchor_along() {
    let source = "ab ![alt](/x.png)\n";
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let (decorations, visual) = decorate(&snapshot, 0).expect("一个块");
    let config = LayoutConfig::new(400.0, LINE_HEIGHT);
    let plain =
        BlockView::build(&visual, &decorations, config, &MonospaceMetrics::new(1.0)).expect("排版");
    let anchor = plain.layout().widgets()[0].visual();

    // 把 "ab" 换成三个字节的 preedit：锚点要往后挪一个字节。
    let replacement = TextRange::new(ByteOffset::ZERO, ByteOffset::new(2)).expect("升序");
    let composed = visual
        .with_composition(replacement, "abc", TextRange::empty(ByteOffset::ZERO))
        .expect("preedit");
    let view = BlockView::build(&composed, &decorations, config, &MonospaceMetrics::new(1.0))
        .expect("排版");
    assert_eq!(
        view.layout().widgets()[0].visual().get(),
        anchor.get() + 1,
        "preedit 长了一个字节，它后面的 widget 锚点也要挪一个字节"
    );
}
