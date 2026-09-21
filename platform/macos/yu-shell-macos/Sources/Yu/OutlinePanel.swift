import AppKit
import CoreText
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
    let depth: Int
    private(set) var children: [OutlineNode] = []
    private(set) weak var parent: OutlineNode?

    /// 面板上显示的那一行文字。
    var label: String { item.label }
    /// 跨刷新的身份。
    var identity: String { item.identity }

    fileprivate init(item: NativeOutlineItem, depth: Int) {
        self.item = item
        self.depth = depth
    }

    fileprivate func append(_ child: OutlineNode) {
        child.parent = self
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
            let node = OutlineNode(item: item, depth: stack.count)
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
private final class NativeOutlineView: NSOutlineView {
    // AppKit can change selection while removing descendant rows. Keep those
    // changes inside the hierarchy operation, rather than navigating the editor.
    var hierarchyWillChange: (() -> Void)?
    var hierarchyDidChange: (() -> Void)?

    override func collapseItem(_ item: Any?, collapseChildren: Bool) {
        hierarchyWillChange?()
        defer { hierarchyDidChange?() }
        super.collapseItem(item, collapseChildren: collapseChildren)
    }

    override func expandItem(_ item: Any?, expandChildren: Bool) {
        hierarchyWillChange?()
        defer { hierarchyDidChange?() }
        super.expandItem(item, expandChildren: expandChildren)
    }

    private var heightRefreshPending = false
    private func scheduleHeightRefresh() {
        guard !heightRefreshPending else { return }
        heightRefreshPending = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.heightRefreshPending = false
            if self.numberOfRows > 0 {
                self.noteHeightOfRows(withIndexesChanged: IndexSet(integersIn: 0..<self.numberOfRows))
            }
        }
    }
    override func setFrameSize(_ newSize: NSSize) {
        let changed = abs(frame.width - newSize.width) > 0.01
        super.setFrameSize(newSize)
        if changed, numberOfRows > 0 {
            scheduleHeightRefresh()
        }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        if numberOfRows > 0 {
            scheduleHeightRefresh()
        }
    }

    override func frameOfCell(atColumn column: Int, row: Int) -> NSRect {
        var frame = super.frameOfCell(atColumn: column, row: row)
        guard column == 0, row >= 0, !frame.isEmpty else { return frame }
        let leading = CGFloat(16) + CGFloat(level(forRow: row)) * indentationPerLevel
        frame.size.width = max(0, frame.maxX - leading)
        frame.origin.x = leading
        return frame
    }

    override func frameOfOutlineCell(atRow row: Int) -> NSRect {
        var frame = super.frameOfOutlineCell(atRow: row)
        guard !frame.isEmpty else { return frame }
        frame.origin.x = 2 + CGFloat(level(forRow: row)) * indentationPerLevel
        return frame
    }
}

private enum OutlineTypography {
    static func lineHeight(dark: Bool) -> CGFloat { dark ? 26 : 22 }

    static func label(_ text: String, runs: [NativeOutlineStyleRun], dark: Bool, active: Bool = false) -> NSAttributedString {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineBreakMode = .byWordWrapping
        paragraph.minimumLineHeight = lineHeight(dark: dark)
        paragraph.maximumLineHeight = lineHeight(dark: dark)
        let result = NSMutableAttributedString(string: text, attributes: [
            .font: NativeTheme.font(identity: NativeTheme.spec(dark: dark).body_font, size: 14),
            .foregroundColor: NSColor.labelColor,
            .paragraphStyle: paragraph,
        ])
        let theme = NativeTheme.spec(dark: dark)
        for run in runs where run.range.length > 0 {
            guard run.range.location >= 0, NSMaxRange(run.range) <= result.length else { continue }
            let code = run.traits & 4 != 0
            let size: CGFloat = code ? 14 * CGFloat(theme.code_size_ratio) : 14
            var font = NativeTheme.font(identity: code ? theme.code_font : theme.body_font, size: size)
            let manager = NSFontManager.shared
            if active || run.traits & 1 != 0 { font = manager.convert(font, toHaveTrait: .boldFontMask) }
            if run.traits & 2 != 0 { font = manager.convert(font, toHaveTrait: .italicFontMask) }
            var attributes: [NSAttributedString.Key: Any] = [.font: font]
            let actual = manager.traits(of: font)
            if run.traits & 2 != 0 && !actual.contains(.italicFontMask) {
                attributes[.obliqueness] = tan(14 * CGFloat.pi / 180)
            }
            if (active || run.traits & 1 != 0) && !actual.contains(.boldFontMask) {
                attributes[.strokeWidth] = -100.0 / 36
                attributes[.kern] = size / 36
            }
            result.addAttributes(attributes, range: run.range)
        }
        return result
    }

    static func height(_ text: String, runs: [NativeOutlineStyleRun], width: CGFloat, dark: Bool, active: Bool = false) -> CGFloat {
        let line = lineHeight(dark: dark)
        let value = label(text, runs: runs, dark: dark, active: active)
        let typesetter = CTTypesetterCreateWithAttributedString(value)
        var offset = 0
        var lines = 0
        while offset < value.length {
            var count = CTTypesetterSuggestLineBreak(typesetter, offset, Double(max(1, width)))
            if count == 0 {
                count = CTTypesetterSuggestClusterBreak(typesetter, offset, Double(max(1, width)))
            }
            offset += max(1, count)
            lines += 1
        }
        return CGFloat(max(1, lines)) * line
    }
}

private final class NativeOutlineCell: NSTableCellView {
    var textTopConstraint: NSLayoutConstraint?
    var styleRuns: [NativeOutlineStyleRun] = []
    var active = false
    func updateTypography() {
        let dark = effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        guard let textField else { return }
        textField.attributedStringValue = OutlineTypography.label(textField.stringValue, runs: styleRuns, dark: dark, active: active)
        textTopConstraint?.constant = dark ? -4 : 0
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        updateTypography()
    }
}

private final class NativeOutlineRow: NSTableRowView {
    override func drawSelection(in dirtyRect: NSRect) {}
}

final class OutlinePanel: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate {
    let scrollView = NSScrollView()
    /// 区头（「大纲」+ 条目计数）。面板自己持有：计数是 reload 的派生物，
    /// 放进面板里才不会在窗口另存一份可以对不上的状态。
    let sectionHeader = YuSidebarSectionHeader(title: "大纲")
    private let outlineView = NativeOutlineView()
    private var roots: [OutlineNode] = []
    private var orderedNodes: [OutlineNode] = []
    private var activeIdentity: String?
    private var activeSourcePosition: Int?
    /// 点了某一条之后要做的事。程序化恢复选中时不触发（见 `restoringSelection`）。
    var onSelect: ((NativeOutlineItem) -> Void)?
    private var restoringSelection = false
    private var hierarchyChangeDepth = 0

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
        outlineView.hierarchyWillChange = { [weak self] in
            self?.hierarchyChangeDepth += 1
        }
        outlineView.hierarchyDidChange = { [weak self] in
            guard let self else { return }
            self.hierarchyChangeDepth -= 1
            if self.hierarchyChangeDepth == 0, !self.restoringSelection,
               let position = self.activeSourcePosition {
                self.highlightHeading(containing: position)
            }
        }
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
        let wasRestoring = restoringSelection
        restoringSelection = true
        defer { restoringSelection = wasRestoring }
        let previouslyKnown = Set(allNodes(of: roots).map(\.identity))
        let previouslyExpanded = Set(
            allNodes(of: roots)
                .filter { outlineView.isItemExpanded($0) }
                .map(\.identity)
        )
        let previouslySelected = selectedNode?.identity

        roots = OutlineTree.build(items: items)
        orderedNodes = allNodes(of: roots)
        outlineView.reloadData()
        // 区头计数跟着这一版大纲走。
        sectionHeader.count = items.count

        for node in allNodes(of: roots) where !node.children.isEmpty {
            if previouslyExpanded.contains(node.identity)
                || !previouslyKnown.contains(node.identity) {
                outlineView.expandItem(node)
            }
        }

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
        if let position = activeSourcePosition {
            highlightHeading(containing: position)
        }
    }

    /// Uses Rust-provided heading source ranges; no Markdown parsing or
    /// navigation occurs when the caret changes.
    func highlightHeading(containing position: Int) {
        activeSourcePosition = position
        var low = 0
        var high = orderedNodes.count
        while low < high {
            let middle = low + (high - low) / 2
            if orderedNodes[middle].item.sourceRange.location <= position { low = middle + 1 }
            else { high = middle }
        }
        var visibleNode = low > 0 ? orderedNodes[low - 1] : nil
        while let node = visibleNode, outlineView.row(forItem: node) < 0 {
            // AppKit forgets parent(forItem:) for rows removed by collapse.
            // Keep the Rust-provided hierarchy available even when hidden.
            visibleNode = node.parent
        }
        let next = visibleNode?.identity
        guard next != activeIdentity else { return }
        let previous = activeIdentity
        activeIdentity = next
        var changed = IndexSet()
        for node in orderedNodes where node.identity == previous || node.identity == next {
            let row = outlineView.row(forItem: node)
            guard row >= 0 else { continue }
            changed.insert(row)
            if let cell = outlineView.view(atColumn: 0, row: row, makeIfNecessary: false) as? NativeOutlineCell {
                cell.active = node.identity == next
                cell.updateTypography()
            }
        }
        if !changed.isEmpty { outlineView.noteHeightOfRows(withIndexesChanged: changed) }
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
        let cell: NativeOutlineCell
        if let reused = outlineView.makeView(withIdentifier: identifier, owner: self)
            as? NativeOutlineCell {
            cell = reused
        } else {
            cell = NativeOutlineCell()
            cell.identifier = identifier
            let field = NSTextField(labelWithString: "")
            field.lineBreakMode = .byWordWrapping
            field.maximumNumberOfLines = 0
            field.cell?.wraps = true
            field.cell?.isScrollable = false
            field.translatesAutoresizingMaskIntoConstraints = false
            field.setAccessibilityElement(false)
            cell.addSubview(field)
            cell.textField = field
            let vertical = field.topAnchor.constraint(equalTo: cell.topAnchor)
            cell.textTopConstraint = vertical
            NSLayoutConstraint.activate([
                field.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 2.0),
                field.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -8.0),
                vertical,
            ])
        }
        cell.active = node.identity == activeIdentity
        cell.styleRuns = node.item.styleRuns
        cell.textField?.stringValue = node.label
        cell.textField?.toolTip = node.label
        // Heading hierarchy is conveyed by native indentation/disclosure, as in
        // the fixed Typora outline reference, rather than document-style icons.
        cell.updateTypography()
        return cell
    }

    func outlineView(_ outlineView: NSOutlineView, heightOfRowByItem item: Any) -> CGFloat {
        guard let node = item as? OutlineNode else { return outlineView.rowHeight }
        let dark = outlineView.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
        let leading = 16 + CGFloat(node.depth) * outlineView.indentationPerLevel
        // Same text insets as the actual cell, including NSTextField's 2pt inset per edge.
        let width = (outlineView.outlineTableColumn?.width ?? outlineView.bounds.width) - leading - 14
        let height = OutlineTypography.height(node.label, runs: node.item.styleRuns, width: width, dark: dark, active: node.identity == activeIdentity)
        return height + 8.5
    }

    func outlineView(_ outlineView: NSOutlineView, rowViewForItem item: Any) -> NSTableRowView? {
        NativeOutlineRow()
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard !restoringSelection, hierarchyChangeDepth == 0, let node = selectedNode else { return }
        highlightHeading(containing: node.item.labelRange.location)
        onSelect?(node.item)
    }

    // MARK: - self-check 入口
    //
    // 面板的判据不能来自面板自己那条路（「条数与 FFI 一致」是自证的），
    // 所以这里只交出 NSOutlineView 眼里的行与树，断言写在 SelfChecks.swift。

    var rootsForSelfCheck: [OutlineNode] { roots }

    func attributedLabelForSelfCheck(_ item: NativeOutlineItem, dark: Bool, active: Bool = false) -> NSAttributedString {
        OutlineTypography.label(item.label, runs: item.styleRuns, dark: dark, active: active)
    }

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

    func rowHeightsForSelfCheck(width: CGFloat, dark: Bool) -> [CGFloat] {
        outlineView.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        outlineView.setFrameSize(NSSize(width: width, height: 1000))
        outlineView.outlineTableColumn?.width = width
        outlineView.noteHeightOfRows(withIndexesChanged: IndexSet(integersIn: 0..<outlineView.numberOfRows))
        return (0..<outlineView.numberOfRows).map { outlineView.rect(ofRow: $0).height }
    }

    var activeIdentityForSelfCheck: String? { activeIdentity }

    func expandForSelfCheck(identity: String) {
        guard let node = orderedNodes.first(where: { $0.identity == identity }) else { return }
        outlineView.expandItem(node)
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
