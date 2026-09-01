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
        .decorate(
            snapshot,
            &tree,
            markdown.reference_definitions(),
            block,
            None,
        )
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
            // 表格不走这一条。一条 `BlockLine` 在表格里是一个**网格行**，
            // 上面的 caret 位置分处几个格子、几个 y；「这一行上最近的
            // caret」在那里不是对的判据——点在第二列，欧氏距离可能把它判给
            // 第一列里换行后更靠近的那个位置，而正确答案是留在第二列。
            // 表格由下面那条按**格**算的性质守着，严格程度一样。
            if view.table().is_some() {
                continue;
            }
            let at = format!("语料 {source:?} 宽度 {width}");
            for line in view.lines() {
                let y = line.y() + line.height() * 0.5;
                // 这一行上够得着的 caret 位置。取簇的两端而不是 `hit_test`
                // 自己的输出——判据不能来自被测的那条路。
                let mut carets: Vec<LayoutPoint> = Vec::new();
                for index in line.cluster_range() {
                    let cluster = view.clusters()[index];
                    for (visual, bias) in [
                        (cluster.visual().start(), Bias::After),
                        (cluster.visual().end(), Bias::Before),
                    ] {
                        if let Ok(caret) = view.caret_for_visual(visual, bias)
                            && caret.line() == line.index()
                        {
                            carets.push(caret.point());
                        }
                    }
                }
                if carets.is_empty() {
                    continue;
                }

                let mut probes: Vec<f32> = carets.iter().map(|caret| caret.x()).collect();
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
                        .map(|caret| (caret.x() - x).abs())
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

/// 表格里点一下，光标停在**点到的那一格**里离点击处最近的 caret 上。
///
/// 与文字流那条是同一句话，只是定义域从「一行」换成「一格」——表格的一行
/// 是一个网格行，上面的位置分处几个格子。此前这里只压得住「不跑到别的行
/// 去」，因为每一格没有自己的布局，命中走的是一段手写的按 x 扫描。现在
/// 每一格有自己的 `BlockLayout`，命中就是它的 `hit`。
#[test]
fn a_table_hit_lands_on_the_nearest_caret_inside_its_cell() {
    let mut checked = 0_usize;
    for source in CORPUS {
        for width in WIDTHS {
            let Some(view) = view(source, *width) else {
                continue;
            };
            let Some(table) = view.table() else {
                continue;
            };
            let at = format!("语料 {source:?} 宽度 {width}");
            for cell in table.cells() {
                // 这一格里够得着的 caret 位置。判据来自簇的两端经
                // `caret_for_visual`，与被测的 `hit_test` 无关。
                let mut carets: Vec<LayoutPoint> = Vec::new();
                for cluster in view.clusters() {
                    if cluster.source().start() < cell.source().start()
                        || cluster.source().end() > cell.source().end()
                    {
                        continue;
                    }
                    for (visual, bias) in [
                        (cluster.visual().start(), Bias::After),
                        (cluster.visual().end(), Bias::Before),
                    ] {
                        if let Ok(caret) = view.caret_for_visual(visual, bias) {
                            carets.push(caret.point());
                        }
                    }
                }
                if carets.is_empty() {
                    continue;
                }
                checked += 1;
                let bounds = cell.bounds();
                let mut probes: Vec<LayoutPoint> = carets.clone();
                for (x, y) in [
                    (bounds.x() + 0.1, bounds.y() + 0.1),
                    (
                        bounds.x() + bounds.width() * 0.5,
                        bounds.y() + bounds.height() * 0.5,
                    ),
                    (
                        bounds.x() + bounds.width() - 0.1,
                        bounds.y() + bounds.height() - 0.1,
                    ),
                ] {
                    probes.push(LayoutPoint::new(x, y));
                }
                for probe in probes {
                    // 探针夹回这一格里：格外的点归别的格，那是另一条性质。
                    let point = LayoutPoint::new(
                        probe
                            .x()
                            .clamp(bounds.x() + 0.1, bounds.x() + bounds.width() - 0.1),
                        probe
                            .y()
                            .clamp(bounds.y() + 0.1, bounds.y() + bounds.height() - 0.1),
                    );
                    let hit = view
                        .hit_test(point)
                        .unwrap_or_else(|error| panic!("{at} 的 hit: {error}"));
                    if hit.image().is_some() {
                        continue;
                    }
                    let distance = |caret: LayoutPoint| {
                        ((caret.x() - point.x()).powi(2) + (caret.y() - point.y()).powi(2)).sqrt()
                    };
                    let got = distance(hit.point());
                    let nearest = carets
                        .iter()
                        .copied()
                        .map(distance)
                        .fold(f32::INFINITY, f32::min);
                    assert!(
                        got <= nearest + 0.001,
                        "{at} 第 {} 行第 {} 列，点 ({}, {})：光标停在 {:?}（差 {got}），\
                         而这一格里最近的 caret 只差 {nearest}",
                        cell.row(),
                        cell.column(),
                        point.x(),
                        point.y(),
                        hit.point()
                    );
                }
            }
        }
    }
    assert!(checked > 0, "语料里要有表格单元格才算压住了什么");
}

/// 点在一个字的左半边，光标停在它**前面**。
///
/// 只断左半边：右半边落在一格的最后一个字上时，「下一个位置」是下一格的
/// 开头——那是对的，但它不再是这一格里的位置，这条断言说不了。
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
                // 探针的 y 取**这个簇自己那一行**，不是网格行的中线：格子里
                // 的内容会换行，网格行的中线可能落在另一行文字上。此前这里
                // 靠「一行里的簇 x 递增」跳过窄表格——列重叠时那个前提不
                // 成立——而列重叠正是这一刀修掉的东西，跳过等于不压。
                for index in line.cluster_range() {
                    let cluster = view.clusters()[index];
                    if cluster.is_line_break() || cluster.width() <= 0.0 {
                        continue;
                    }
                    let y = cluster.y() + view.config().line_height() * 0.5;
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
        .decorate(
            &snapshot,
            &tree,
            markdown.reference_definitions(),
            block,
            Some(active),
        )
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
    let Some(yu_editor::BlockWidget::Image(image)) = decorations.widgets().first().copied() else {
        panic!("这个块上有一张图");
    };
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

/// 表格格子里的图片要排出一个落在**那一格**里的盒子。
///
/// 上面那条只压得住「列被撑宽了」——它走的是 `measure_cell_content`，直接
/// 读整块的 widget 锚点，不经过格子的切片。切片忘了带 widget 的话列照样撑
/// 宽，而格内一张图都排不出来：`images()` 是空的，图整个不画，不报错。
#[test]
fn an_image_in_a_cell_gets_a_box_inside_that_cell() {
    let buffer = TextBuffer::new("| a | ![i](/x.png) |\n| --- | --- |\n| 1 | 2 |\n".to_owned());
    let snapshot = buffer.snapshot();
    let (decorations, visual) = decorate(&snapshot, 0).expect("一个块");
    let view = BlockView::build(
        &visual,
        &decorations,
        LayoutConfig::new(400.0, LINE_HEIGHT),
        &MonospaceMetrics::new(1.0),
    )
    .expect("排版");
    let table = view.table().expect("这个块是一张表");
    assert_eq!(view.images().len(), 1, "格子里那张图要有盒子");

    let placement = view.images()[0];
    let cell = table
        .cells()
        .iter()
        .copied()
        .find(|cell| {
            cell.source().start() <= placement.source().start()
                && placement.source().end() <= cell.source().end()
        })
        .expect("盒子要属于某一格");
    let (bounds, cell_bounds) = (placement.bounds(), cell.bounds());
    assert!(
        bounds.x() >= cell_bounds.x() - 0.001
            && bounds.x() + bounds.width() <= cell_bounds.x() + cell_bounds.width() + 0.001,
        "图片盒子 {bounds:?} 横向越出了单元格 {cell_bounds:?}"
    );
    assert!(
        bounds.y() >= cell_bounds.y() - 0.001
            && bounds.y() + bounds.height() <= cell_bounds.y() + cell_bounds.height() + 0.001,
        "图片盒子 {bounds:?} 纵向越出了单元格 {cell_bounds:?}"
    );
}

// ---------------------------------------------------------- 复选框 widget

/// 复选框在行里**占位**，不压在正文上。
///
/// 这是它从 `Decoration::Replace` 换成 `Decoration::Widget` 的全部理由。
/// `Replace` 让 `[x]` 的视觉宽度变成零，整段塌成一个点，而方框是画在那个点
/// 上的一个**有宽度**的覆盖物——于是它盖住正文的第一个字。截图一眼就看得
/// 见，而当时所有断言都是绿的：盒子在、几何自洽、id 查得到。
///
/// 判据因此是**几何**而不是「有没有一个 widget」：框右沿之后才允许有簇。
#[test]
fn a_checkbox_reserves_its_own_box_instead_of_sitting_on_the_text() {
    let view = view("- [x] 待办", 200.0).expect("一个任务项块");
    let boxes = view.checkboxes();
    assert_eq!(boxes.len(), 1, "一个任务项一个复选框，实际是 {boxes:?}");
    let bounds = boxes[0].bounds();
    assert!(bounds.width() > 0.0, "复选框必须有宽度，否则它没有占位");
    assert_eq!(bounds.width(), bounds.height(), "复选框是正方形");
    // 边长钉死在这里，与图片的 placeholder 宽度同一个待遇：常数没有断言
    // 就等于没有约定，下一个人改它不会有任何东西变红。
    assert_eq!(bounds.width(), 10.0 * 0.68, "复选框是 0.68 个行高");

    let right = bounds.x() + bounds.width();
    for cluster in view.clusters() {
        assert!(
            cluster.x() >= right || cluster.x() + cluster.width() <= bounds.x(),
            "簇 {:?} 落在复选框 {bounds:?} 里，方框会压在字上",
            cluster.source()
        );
    }
}

/// 勾没勾上、盖住哪三个字节，都由装饰带过来。
#[test]
fn a_checkbox_carries_its_state_and_the_three_bytes_it_covers() {
    let todo = view("- [ ] 待办", 200.0).expect("块");
    assert_eq!(todo.checkboxes()[0].state(), yu_markdown::TaskState::Todo);
    assert_eq!(
        todo.checkboxes()[0].source(),
        TextRange::new(ByteOffset::new(2), ByteOffset::new(5)).expect("区间")
    );

    let done = view("- [X] 完成", 200.0).expect("块");
    assert_eq!(done.checkboxes()[0].state(), yu_markdown::TaskState::Done);

    let plain = view("- 项目", 200.0).expect("块");
    assert!(plain.checkboxes().is_empty(), "普通列表项没有复选框");
}

/// 复选框永远是 `Ready`，一次都不进 `pending_widgets`（不变量 D7）。
///
/// 报成 `Placeholder` 不会画错，只会让 `LayoutCache` 永远认为「还欠着一个
/// 资源」——于是每一帧重排一次这个块。这种退化不报错，只是慢。
#[test]
fn a_checkbox_never_waits_for_a_resource() {
    let view = view("- [x] 待办", 200.0).expect("块");
    assert!(
        view.layout().pending_widgets().is_empty(),
        "复选框的尺寸只依赖行高，没有要等的东西"
    );
}

/// 点在复选框上落到它的两沿，不落进它里面。
///
/// 与图片走的是**同一条**规则（`PlacedWidget::hit`）：盒子有宽度，两沿差着
/// 整个盒子，只有排它的人知道点落在哪一沿。抄第二遍就会分叉，而分叉的表现
/// 是「光标画在一处、点击落在另一处」。
#[test]
fn clicking_a_checkbox_lands_on_one_of_its_edges() {
    let view = view("- [x] 待办", 200.0).expect("块");
    let bounds = view.checkboxes()[0].bounds();
    let y = bounds.y() + bounds.height() * 0.5;

    let left = view
        .hit_test(yu_layout::LayoutPoint::new(
            bounds.x() + bounds.width() * 0.25,
            y,
        ))
        .expect("命中");
    let right = view
        .hit_test(yu_layout::LayoutPoint::new(
            bounds.x() + bounds.width() * 0.75,
            y,
        ))
        .expect("命中");
    assert_eq!(left.source(), view.checkboxes()[0].source().start());
    assert_eq!(right.source(), view.checkboxes()[0].source().end());
    // 两沿的答案来自 `BlockLayout::hit` 的 `widget_affinity`（第七刀），
    // 不来自图片那条命中快路——复选框**不**在那条路上。串进去不只是多余：
    // `image()` 会把一次复选框点击报成「点在一张图上」，而 FFI 照着它给
    // 平台一个图片区间。
    assert_eq!(left.image(), None, "复选框不是图片");
    assert_eq!(right.image(), None, "复选框不是图片");
}

/// 块整体平移时复选框跟着走。
///
/// 缓存把一个块的排版结果按 `delta` 平移复用（编辑发生在它前面时）。漏平移
/// 一种 widget 不会 panic：盒子还在、几何还自洽，只是它指着**平移前**的那
/// 三个字节。表现是点一下复选框，改的是另一处的源码。
#[test]
fn a_prefix_edit_moves_the_checkbox_with_its_block() {
    let source = "para\n\n- [x] 待办\n";
    let mut buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let (decorations, visual) = decorate(&snapshot, 2).expect("任务项那个块");
    let config = LayoutConfig::new(200.0, 10.0).with_default_advance(2.0);
    let view = BlockView::build(
        &visual,
        &decorations,
        config,
        &MonospaceMetrics::new(config.default_advance()),
    )
    .expect("BlockView");
    let before = view.checkboxes().first().copied().expect("有一个复选框");

    let transaction = yu_text::Transaction::new(
        snapshot.revision(),
        [yu_text::Edit::new(TextRange::empty(ByteOffset::ZERO), "xx")],
    );
    let applied = buffer.apply(&transaction).expect("插入");
    let after_snapshot = applied.result_snapshot().clone();
    let mapped = view
        .map_through(applied.change_set(), &after_snapshot)
        .expect("重映射")
        .expect("块仍然存在");
    let after = mapped.checkboxes().first().copied().expect("还是有一个");

    assert_eq!(
        after.source().start().get(),
        before.source().start().get() + 2
    );
    assert_eq!(after.source().end().get(), before.source().end().get() + 2);
    assert_eq!(after.bounds(), before.bounds(), "块外的编辑不该动几何");
}

/// 一个等宽的测试 shaper：一个 grapheme 一个字形，宽度恒为 2。
///
/// 高亮的角色只有在**字形**上才看得见（`BlockGlyph`），而
/// `MonospaceMetrics` 那条路只量宽度、不产字形。用例因此必须走
/// `build_shaped`。这个 shaper 与 `MonospaceMetrics::new(2.0)` 的度量一致，
/// 所以它不引入第二套几何。
#[derive(Clone, Copy, Debug)]
struct TestShaper;

impl yu_core::ShapingProvider for TestShaper {
    type Error = &'static str;

    fn shape(
        &self,
        text: &str,
        source: TextRange,
        style: yu_core::TextStyle,
    ) -> Result<yu_core::ShapedText, Self::Error> {
        use unicode_segmentation::UnicodeSegmentation as _;
        let glyphs = text
            .grapheme_indices(true)
            .map(|(start, cluster)| {
                let from = source.start().get() + start as u64;
                let to = from + cluster.len() as u64;
                let range =
                    TextRange::new(ByteOffset::new(from), ByteOffset::new(to)).expect("升序");
                yu_core::Glyph::new(yu_core::GlyphId::from_raw(1), range, 2.0, 0.0, 0.0)
            })
            .collect();
        Ok(yu_core::ShapedText::new(
            source,
            vec![yu_core::GlyphRun::new(
                yu_core::FontFaceId::from_raw(1),
                source,
                style,
                yu_core::TextDirection::Ltr,
                yu_core::Script::Latin,
                glyphs,
            )],
        ))
    }
}

#[test]
fn the_test_shaper_conforms_to_the_shaping_contract() {
    let violations = yu_core::shaping_conformance::audit(&TestShaper);
    assert!(violations.is_empty(), "{violations:#?}");
}

fn shaped_view(source: &str, width: f32) -> Option<BlockView> {
    let buffer = TextBuffer::new(source.to_owned());
    let snapshot = buffer.snapshot();
    let (decorations, visual) = decorate(&snapshot, 0)?;
    let config = LayoutConfig::new(width, 10.0).with_default_advance(2.0);
    Some(BlockView::build_shaped(&visual, &decorations, config, &TestShaper).expect("BlockView"))
}

/// 高亮的角色跟着字形一直走到 `BlockGlyph`。
///
/// 判据是**字形自己的源码区间**：把 `glyph.source()` 切出来跟 `fn` / `u32`
/// 比，而不是数「有几个字形带角色」。第四刀在这件事上付过两次——只数条数压
/// 不住「指错了哪一处」。
#[test]
fn code_highlight_roles_reach_the_glyphs() {
    let source = "```rust\nfn main() {\n    let x: u32 = 1;\n}\n```\n";
    let view = shaped_view(source, 400.0).expect("代码块有 BlockView");
    let text_of = |glyph: &yu_editor::BlockGlyph| {
        let start = usize::try_from(glyph.source().start().get()).expect("短");
        let end = usize::try_from(glyph.source().end().get()).expect("短");
        source[start..end].to_owned()
    };
    let with_role = |role: yu_core::TextRole| {
        view.glyphs()
            .iter()
            .filter(|glyph| glyph.role() == role)
            .map(text_of)
            .collect::<Vec<_>>()
    };
    // 等宽度量下一个 ASCII 字符一个字形，所以关键字 `fn` 是两个字形。
    assert_eq!(
        with_role(yu_core::TextRole::Keyword),
        vec!["f", "n", "l", "e", "t"]
    );
    assert_eq!(with_role(yu_core::TextRole::Type), vec!["u", "3", "2"]);
    // 角色不改变字面：代码块里的每一个字形都还是等宽的。
    for glyph in view.glyphs() {
        assert_eq!(
            glyph.style(),
            yu_core::TextStyle::Code,
            "{:?} 掉出了等宽字面",
            text_of(glyph)
        );
    }
}

/// 没有高亮的块，每一个字形都是 `TextRole::Plain`。
///
/// 与上一条合起来才说明「角色是加上去的」：只有上一条的话，一个把所有字形
/// 都标成 `Keyword` 的实现也能过。
#[test]
fn blocks_without_highlight_carry_no_role() {
    for source in [
        "plain paragraph\n",
        "# h1 title\n",
        "```\nfn main() {}\n```\n",
        // **普通列表项**，不是任务项：只有前者会产出替代标记（`•`），而标记的
        // 字形不在 source 里、没有任何 Mark 盖着它，是全场唯一一条不查样式表
        // 就写死角色的路。语料里只放任务项的话那一行根本走不到——「给标记也
        // 编一个角色」这个变异因此活过一次。
        "- item one\n",
        "- [x] done item\n",
        "text with `code` and **strong**\n",
    ] {
        let Some(view) = shaped_view(source, 400.0) else {
            continue;
        };
        assert!(!view.glyphs().is_empty(), "语料 {source:?} 该排出字形");
        for glyph in view.glyphs() {
            assert_eq!(
                glyph.role(),
                yu_core::TextRole::Plain,
                "语料 {source:?} 不该有角色"
            );
        }
    }
}
