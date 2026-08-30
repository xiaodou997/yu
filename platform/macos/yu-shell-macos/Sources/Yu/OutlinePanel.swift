import AppKit
import Foundation
import YuStorageFFI

// 大纲面板：把 yu_storage_session_outline_items 的平表画成一列可点的标题。
//
// 这里只有三件事是真正的 Swift 逻辑，其余都是 AppKit 样板：
//
//   1. **平表 → 树**。FFI 给的是带 `parent` 下标的平表，NSOutlineView 要
//      parent→children。挂错父亲不报错、不 panic，面板照样画得出来，只是
//      层级是错的——self-check 因此拿树的形状反过来核对平表。
//   2. **跨刷新的身份**。NSOutlineView 按对象身份记展开状态，而每次刷新都会
//      重建这些对象。展开了哪几条、选中哪一条要靠 `identity` 存活。
//   3. **label 的折行**。见 `OutlineTree.displayLabel`。
//
// 导航**不另开 FFI**：拿 `labelRange.location` 走已有的选区入口，滚动交给
// yu-editor::viewport 那条路（`macosShapedCaretScrollRequest`）。面板不算 y。

/// 平表转成树之后的一个节点。
///
/// 引用类型是 NSOutlineView 的要求（它按对象身份认 item），不是设计偏好。
final class OutlineNode {
    let item: NativeOutlineItem
    /// 面板上显示的那一行文字，见 `OutlineTree.displayLabel`。
    let label: String
    /// 跨刷新的身份：从根到自己的 label 链，同名兄弟按出现次序区分。
    ///
    /// 不用 `index`：插入一条标题会把它后面所有条目的 index 推后一位，
    /// 展开状态会整体错位。也不用 `block`：同样会被前面的块推移。
    let identity: String
    private(set) var children: [OutlineNode] = []

    fileprivate init(item: NativeOutlineItem, label: String, identity: String) {
        self.item = item
        self.label = label
        self.identity = identity
    }

    fileprivate func append(_ child: OutlineNode) {
        children.append(child)
    }
}

enum OutlineTree {
    /// 平表 → 树。
    ///
    /// 标题按文档顺序排列，`parent` 又只指向上一级标题，所以父亲一定排在
    /// 孩子前面，一遍扫描就够。查不到父亲的那一条挂成根级——那是 FFI 契约
    /// 被破坏的情形，而根节点的 `parent` 必须是 `UInt32.max` 这一条断言会
    /// 把它照出来，不会静默地混进正常层级里。
    static func build(items: [NativeOutlineItem], source: NSString) -> [OutlineNode] {
        var roots: [OutlineNode] = []
        var byIndex: [UInt32: OutlineNode] = [:]
        // key 是「父亲的 identity + label」，值是这个 label 在该父亲下出现过
        // 几次。同一层里两条同名标题靠它区分。
        var occurrences: [String: Int] = [:]
        for item in items {
            let label = displayLabel(item.labelRange, in: source)
            let parent = item.parent == UInt32.max ? nil : byIndex[item.parent]
            let base = (parent?.identity ?? "") + "\u{1F}" + label
            let seen = occurrences[base, default: 0]
            occurrences[base] = seen + 1
            let node = OutlineNode(
                item: item,
                label: label,
                identity: base + "\u{1F}\(seen)"
            )
            byIndex[item.index] = node
            if let parent {
                parent.append(node)
            } else {
                roots.append(node)
            }
        }
        return roots
    }

    /// 面板上的一行文字。
    ///
    /// **这一刀不剥行内标记**：`## **粗** 标题` 显示成 `**粗** 标题`。剥的
    /// 唯一实现在 `DecorationSet` 里（不变量 D1），而 FFI 上没有任何回报视觉
    /// 区间的入口；开一个只为一个消费者用的新 FFI 不划算，触发条件是第三刀的
    /// 搜索面板。理由与触发条件写在 overview 第 8 节 S7
    /// 「已登记：面板上的标题带着行内标记，第三刀再剥」。显示源码不是错的
    /// 答案——它显示的是源码，没有引入第二份定义。
    ///
    /// 唯一的纯呈现例外是 **Setext 多行标题**：`多行\n标题\n===` 的 label
    /// 是 `"多行\n标题"`，一行放不下两行字，这里折成一行。
    static func displayLabel(_ range: NSRange, in source: NSString) -> String {
        guard range.location >= 0,
              range.length >= 0,
              range.location + range.length <= source.length else {
            return ""
        }
        let raw = source.substring(with: range)
        guard raw.contains(where: \.isNewline) else { return raw }
        return raw
            .split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
            .joined(separator: " ")
    }
}

/// 面板本体。持有 NSOutlineView 与它的滚动视图，暴露一个 `reload` 与一个
/// 选中回调；它不认识 StorageBridge，也不认识窗口。
final class OutlinePanel: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate {
    let scrollView = NSScrollView()
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
        outlineView.rowSizeStyle = .default
        outlineView.indentationPerLevel = 14.0
        outlineView.usesAutomaticRowHeights = false
        outlineView.style = .plain
        outlineView.backgroundColor = .controlBackgroundColor
        outlineView.dataSource = self
        outlineView.delegate = self
        outlineView.setAccessibilityLabel("文档大纲")

        scrollView.documentView = outlineView
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = true
        scrollView.backgroundColor = .controlBackgroundColor
        scrollView.translatesAutoresizingMaskIntoConstraints = false
    }

    /// 面板要不要接受键盘焦点由窗口决定；这里只把 outline view 交出去。
    var focusTarget: NSView { outlineView }

    /// 用新一版的大纲重建整棵树。
    ///
    /// **展开状态与选中行必须活过这一次重建**：每次全量 `reloadData` 都会
    /// 换掉所有节点对象，什么都不做的话，敲一个字符大纲就全折起来、选中行
    /// 也没了。新出现的节点默认展开——刚打的标题应该看得见。
    func reload(items: [NativeOutlineItem], source: NSString) {
        let previouslyKnown = Set(allNodes(of: roots).map(\.identity))
        let previouslyExpanded = Set(
            allNodes(of: roots)
                .filter { outlineView.isItemExpanded($0) }
                .map(\.identity)
        )
        let previouslySelected = selectedNode?.identity

        roots = OutlineTree.build(items: items, source: source)
        outlineView.reloadData()

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
        let field: NSTextField
        if let reused = outlineView.makeView(withIdentifier: identifier, owner: self)
            as? NSTextField {
            field = reused
        } else {
            field = NSTextField(labelWithString: "")
            field.identifier = identifier
            field.lineBreakMode = .byTruncatingTail
        }
        field.stringValue = node.label
        field.toolTip = node.label
        // 级别只改字重，不改字号：面板是一列索引，不是文档的缩微图。
        field.font = NSFont.systemFont(
            ofSize: NSFont.systemFontSize,
            weight: node.item.level <= 1 ? .semibold : .regular
        )
        return field
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
