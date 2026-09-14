import AppKit
import Foundation
import YuStorageFFI

// 大纲面板：把 yu_storage_session_outline_items 交下来的一棵树喂给
// NSOutlineView。
//
// **这里只剩 AppKit。** 平表→树、跨刷新的身份链、label 的减法与折行原来都
// 住在这个文件里，第七刀 c 的第三块把它们挪进了 `yu-editor::OutlineTree`：
// 没有一样需要 AppKit，而第二端照写第二遍的表现是同一条标题在两端显示得不
// 一样、展开状态在一端活得下来在另一端活不下来——都不报错。
//
// 剩下的两件事是真正的 AppKit 事实：
//
//   1. **NSOutlineView 要 parent→children 的对象图**，而 C ABI 只能交出平表。
//      还原那棵树在这里，但**它没有决策**：表是前序的，每一条带着自己直接
//      孩子的条数，一次栈式扫描就够，没有查表也没有「父亲查不到怎么办」。
//   2. **它按对象身份记展开状态**，而每次刷新都会重建这些对象。展开了哪几
//      条、选中哪一条要靠 Rust 给的 `identity` 存活。
//
// 导航**不另开 FFI**：拿 `labelRange.location` 走已有的选区入口，滚动交给
// yu-editor::viewport 那条路（`shapedCaretScrollRequest`）。面板不算 y。

/// 树上的一个节点。
///
/// 引用类型是 NSOutlineView 的要求（它按对象身份认 item），不是设计偏好。
final class OutlineNode {
    let item: NativeOutlineItem
    private(set) var children: [OutlineNode] = []

    /// 面板上显示的那一行文字。
    var label: String { item.label }
    /// 跨刷新的身份。
    var identity: String { item.identity }

    fileprivate init(item: NativeOutlineItem) {
        self.item = item
    }

    fileprivate func append(_ child: OutlineNode) {
        children.append(child)
    }
}

enum OutlineTree {
    /// 前序的平表 → 对象图。
    ///
    /// 每一条带着自己**直接**孩子的条数，所以扫一遍就够：栈顶那个还欠孩子
    /// 就挂上去，欠满了就弹掉。层级由 Rust 定，这里一个判断都不做——原来那
    /// 份按 `parent` 查表的写法有一支「查不到父亲就挂成根级」，那是 FFI 契约
    /// 被破坏时的猜测，现在不需要猜了。
    static func build(items: [NativeOutlineItem]) -> [OutlineNode] {
        var roots: [OutlineNode] = []
        // 栈里存 (节点, 还欠几个直接孩子)。
        var stack: [(node: OutlineNode, remaining: Int)] = []
        for item in items {
            while let top = stack.last, top.remaining == 0 {
                stack.removeLast()
            }
            let node = OutlineNode(item: item)
            if let parent = stack.last {
                parent.node.append(node)
                stack[stack.count - 1].remaining -= 1
            } else {
                roots.append(node)
            }
            stack.append((node, item.childCount))
        }
        return roots
    }
}

/// 面板本体。持有 NSOutlineView 与它的滚动视图，暴露一个 `reload` 与一个
/// 选中回调；它不认识 StorageBridge，也不认识窗口。
final class OutlinePanel: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate {
    let scrollView = NSScrollView()
    /// 区头（「大纲」+ 条目计数）。面板自己持有：计数是 reload 的派生物，
    /// 放进面板里才不会在窗口另存一份可以对不上的状态。
    let sectionHeader = YuSidebarSectionHeader(title: "大纲")
    private let outlineView = NSOutlineView()
    private var roots: [OutlineNode] = []
    /// 点了某一条之后要做的事。程序化恢复选中时不触发（见 `restoringSelection`）。
    var onSelect: ((NativeOutlineItem) -> Void)?
    private var restoringSelection = false

    override init() {
        super.init()
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("outline"))
        column.title = "大纲"
        column.resizingMask = .autoresizingMask
        outlineView.addTableColumn(column)
        outlineView.outlineTableColumn = column
        outlineView.headerView = nil
        // rowSizeStyle 必须是 .custom：.medium 会接管 rowHeight（读出来恒为
        // 17），这里以前的 32 实际从未生效。行高与缩进走 token（稿 30 / 14）。
        outlineView.rowSizeStyle = .custom
        outlineView.rowHeight = YuVisualTokens.sidebarRowHeight
        outlineView.indentationPerLevel = YuVisualTokens.sidebarIndent
        outlineView.usesAutomaticRowHeights = false
        outlineView.style = .plain
        outlineView.backgroundColor = .clear
        outlineView.enclosingScrollView?.drawsBackground = false
        outlineView.selectionHighlightStyle = .regular
        outlineView.usesAlternatingRowBackgroundColors = false
        outlineView.dataSource = self
        outlineView.delegate = self
        outlineView.setAccessibilityLabel("文档大纲")

        scrollView.documentView = outlineView
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false
    }

    /// 面板要不要接受键盘焦点由窗口决定；这里只把 outline view 交出去。
    var focusTarget: NSView { outlineView }

    /// 用新一版的大纲重建整棵树。
    ///
    /// **展开状态与选中行必须活过这一次重建**：每次全量 `reloadData` 都会
    /// 换掉所有节点对象，什么都不做的话，敲一个字符大纲就全折起来、选中行
    /// 也没了。新出现的节点默认展开——刚打的标题应该看得见。
    func reload(items: [NativeOutlineItem]) {
        let previouslyKnown = Set(allNodes(of: roots).map(\.identity))
        let previouslyExpanded = Set(
            allNodes(of: roots)
                .filter { outlineView.isItemExpanded($0) }
                .map(\.identity)
        )
        let previouslySelected = selectedNode?.identity

        roots = OutlineTree.build(items: items)
        outlineView.reloadData()
        // 区头计数跟着这一版大纲走。
        sectionHeader.count = items.count

        for node in allNodes(of: roots) where !node.children.isEmpty {
            if previouslyExpanded.contains(node.identity)
                || !previouslyKnown.contains(node.identity) {
                outlineView.expandItem(node)
            }
        }

        restoringSelection = true
        if let previouslySelected,
           let node = allNodes(of: roots).first(where: { $0.identity == previouslySelected }) {
            let row = outlineView.row(forItem: node)
            if row >= 0 {
                outlineView.selectRowIndexes([row], byExtendingSelection: false)
            } else {
                outlineView.deselectAll(nil)
            }
        } else {
            outlineView.deselectAll(nil)
        }
        restoringSelection = false
    }

    private var selectedNode: OutlineNode? {
        let row = outlineView.selectedRow
        guard row >= 0 else { return nil }
        return outlineView.item(atRow: row) as? OutlineNode
    }

    private func allNodes(of nodes: [OutlineNode]) -> [OutlineNode] {
        nodes.flatMap { [$0] + allNodes(of: $0.children) }
    }

    // MARK: - NSOutlineViewDataSource / Delegate

    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        guard let node = item as? OutlineNode else { return roots.count }
        return node.children.count
    }

    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        guard let node = item as? OutlineNode else { return roots[index] }
        return node.children[index]
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        guard let node = item as? OutlineNode else { return false }
        return !node.children.isEmpty
    }

    func outlineView(
        _ outlineView: NSOutlineView,
        viewFor tableColumn: NSTableColumn?,
        item: Any
    ) -> NSView? {
        guard let node = item as? OutlineNode else { return nil }
        let identifier = NSUserInterfaceItemIdentifier("outline-cell")
        let cell: NSTableCellView
        if let reused = outlineView.makeView(withIdentifier: identifier, owner: self)
            as? NSTableCellView {
            cell = reused
        } else {
            cell = NSTableCellView()
            cell.identifier = identifier
            let icon = NSImageView()
            icon.imageScaling = .scaleProportionallyUpOrDown
            icon.translatesAutoresizingMaskIntoConstraints = false
            icon.setAccessibilityElement(false)
            let field = NSTextField(labelWithString: "")
            field.lineBreakMode = .byTruncatingTail
            field.translatesAutoresizingMaskIntoConstraints = false
            field.setAccessibilityElement(false)
            cell.addSubview(icon)
            cell.addSubview(field)
            cell.imageView = icon
            cell.textField = field
            NSLayoutConstraint.activate([
                icon.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 2.0),
                icon.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
                icon.widthAnchor.constraint(equalToConstant: 16.0),
                icon.heightAnchor.constraint(equalToConstant: 16.0),
                field.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 7.0),
                field.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -8.0),
                field.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            ])
        }
        cell.textField?.stringValue = node.label
        cell.textField?.toolTip = node.label
        // 级别只改字重与颜色，不改字号：面板是一列索引，不是文档的缩微图。
        // H1 用 semibold 撑住层级，H4 以下降为次要色。
        cell.textField?.font = NSFont.systemFont(
            ofSize: YuVisualTokens.sidebarItemFontSize,
            weight: node.item.level <= 1 ? .semibold : .regular
        )
        cell.textField?.textColor = node.item.level >= 4
            ? .secondaryLabelColor
            : .labelColor
        // 图标按层级区分语义：H1 大号 text.alignleft，H2/H3 缩进类变体，
        // H4-H6 小号 doc。字号用 symbol configuration 控，格子固定 16×16。
        let symbol: String
        let symbolSize: CGFloat
        switch node.item.level {
        case 1:
            symbol = "text.alignleft"
            symbolSize = 15
        case 2:
            symbol = "list.bullet.indent"
            symbolSize = 14
        case 3:
            symbol = "text.indent"
            symbolSize = 13
        default:
            symbol = "doc.text"
            symbolSize = 11
        }
        cell.imageView?.image = NSImage(
            systemSymbolName: symbol,
            accessibilityDescription: "标题"
        )?.withSymbolConfiguration(
            NSImage.SymbolConfiguration(pointSize: symbolSize, weight: .regular)
        )
        switch node.item.level {
        case 1:
            cell.imageView?.contentTintColor = YuVisualTokens.accent
        case 2, 3:
            cell.imageView?.contentTintColor = .secondaryLabelColor
        default:
            cell.imageView?.contentTintColor = .tertiaryLabelColor
        }
        return cell
    }

    func outlineView(_ outlineView: NSOutlineView, rowViewForItem item: Any) -> NSTableRowView? {
        YuSidebarRowView()
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard !restoringSelection, let node = selectedNode else { return }
        onSelect?(node.item)
    }

    // MARK: - self-check 入口
    //
    // 面板的判据不能来自面板自己那条路（「条数与 FFI 一致」是自证的），
    // 所以这里只交出 NSOutlineView 眼里的行与树，断言写在 SelfChecks.swift。

    var rootsForSelfCheck: [OutlineNode] { roots }

    var rowCountForSelfCheck: Int { outlineView.numberOfRows }

    /// 行高与缩进是视觉结构的一部分：断言写设计稿数值，将来改设计时
    /// 改 token 的同时改这里。
    var rowMetricsForSelfCheck: (height: CGFloat, indent: CGFloat) {
        (outlineView.rowHeight, outlineView.indentationPerLevel)
    }

    var sectionHeaderCountForSelfCheck: Int { sectionHeader.count }

    func nodeForSelfCheck(row: Int) -> OutlineNode? {
        outlineView.item(atRow: row) as? OutlineNode
    }

    func clickRowForSelfCheck(_ row: Int) {
        outlineView.deselectAll(nil)
        outlineView.selectRowIndexes([row], byExtendingSelection: false)
    }

    var selectedIdentityForSelfCheck: String? { selectedNode?.identity }

    var expandedIdentitiesForSelfCheck: Set<String> {
        Set(
            allNodes(of: roots)
                .filter { outlineView.isItemExpanded($0) }
                .map(\.identity)
        )
    }

    func collapseForSelfCheck(identity: String) {
        guard let node = allNodes(of: roots).first(where: { $0.identity == identity }) else {
            return
        }
        outlineView.collapseItem(node)
    }
}
