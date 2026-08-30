import AppKit
import Foundation
import YuStorageFFI

// 搜索面板：一个查询框加一列可点的结果。
//
// # 为什么不复用 OutlinePanel
//
// 两者在「一列可点的条目 → 定位到源码位置」这句话上是同一个形状，具体到代码
// 能搬的不到一半：`NSOutlineViewDataSource` 的 `numberOfChildrenOfItem` /
// `child:ofItem:` / `isItemExpandable` 三个方法 `NSTableView` 一个都用不上；
// `reload` 有一半是展开状态的恢复，而搜索结果**没有展开状态**；平表→树与
// 跨刷新的 identity 链更是完全不需要——结果是平的一列，每换一次查询整体重建，
// 条目的身份就是「第几个匹配」。
//
// 所以各写各的。第二个消费者到了，但它要的不是同一样东西——这是这个项目
// 「不为还没有第二个消费者的东西先建抽象」那条规矩的一个变体。
//
// **但有两样必须共用**，它们各自只能有一个实现：
//
//   1. **导航**——`DocumentTextView.navigate(toSource:)`。另写一份会立刻产生
//      第二个「怎么跳到一个源码位置」的答案，而这一刀恰好又要动选区。
//   2. **拿镜像减区间**——`PanelLabel`。结果那一行同样要显示不带语法标记的
//      文字，两边各写一份必定分叉，表现是同一段文字在两个面板上不一样。

/// 结果列表上的一行。
struct SearchResultRow {
    let match: NativeSearchMatch
    /// 面板上显示的那一行文字：命中所在的那一行，剥掉语法标记。
    let label: String
}

enum SearchResults {
    /// 一处命中在面板上显示成哪一行字。
    ///
    /// 取「命中所在的那一行」，再**裁进它所在的块**——回报隐藏区间的 FFI 只
    /// 接受落在一个块里的请求（跨块入口会逼那一层去回答「块边界在哪」，那是
    /// 上一层的事）。行与块的边界大多数时候重合，不重合的是列表项、引用块
    /// 这些容器里的行。
    ///
    /// 拿不到区间时退回显示源码——那是一件真事，不是错的答案。
    static func row(
        for match: NativeSearchMatch,
        in source: NSString,
        hidden: (UInt64, NSRange) -> [NSRange]?
    ) -> SearchResultRow {
        let context = contextRange(for: match, in: source)
        let spans = hidden(match.block, context)
        let label = PanelLabel.stripping(spans ?? [], from: source, in: context)
        return SearchResultRow(
            match: match,
            label: label.trimmingCharacters(in: .whitespacesAndNewlines)
        )
    }

    /// 「跳到下一个/上一个」：以**选区**为游标，环回。
    ///
    /// 游标是选区而不是一个存下来的下标：存下标就有两个可以对不上的答案，
    /// 而选区是导航必然会更新的那一份，Rust 侧的「当前命中」也从它推出来。
    /// 于是在文档里点一下再按下一个，走的是点的位置——那正是该有的行为。
    ///
    /// 比的是起点：光标停在某处命中的起点上时，「下一个」是它后面那一处。
    static func next(
        after selection: NSRange,
        in matches: [NativeSearchMatch],
        forward: Bool
    ) -> NativeSearchMatch? {
        guard !matches.isEmpty else { return nil }
        if forward {
            return matches.first { $0.range.location > selection.location } ?? matches.first
        }
        return matches.last { $0.range.location < selection.location } ?? matches.last
    }

    /// 命中所在的那一行 ∩ 它所在的块。
    ///
    /// **今天这个交集取不出东西来**：块的边界是按行划的，所以一行必然落在一个
    /// 块里。留着它有两个理由，而不是当死代码删掉：
    ///
    ///   1. `NSString.lineRange(for:)` 认的是 Unicode 的行边界（`\u{2028}`、
    ///      `\u{2029}`、`\r` 都算），而块扫描器只认 `\n`。两者对「一行」的
    ///      定义本来就不是同一个。
    ///   2. **「块的边界还没合并」是一条已登记的欠账**（见 overview 的
    ///      「块结构合并：调查结论」）。那道闸门一旦打开，块会下降到容器里，
    ///      「一行落在一个块里」就不再成立。
    ///
    /// 不裁的后果不是画错，是**静默地不剥**：请求跨出块，回报隐藏区间的 FFI
    /// 直接拒绝，那一行悄悄带回语法标记。所以它由一条手造输入压着
    /// （self-check 里那条「块比行窄」），不靠语料碰运气。
    static func contextRange(for match: NativeSearchMatch, in source: NSString) -> NSRange {
        guard match.range.location >= 0, match.range.location <= source.length else {
            return NSRange(location: 0, length: 0)
        }
        let line = source.lineRange(
            for: NSRange(location: min(match.range.location, max(source.length - 1, 0)), length: 0)
        )
        return NSIntersectionRange(line, match.blockRange)
    }
}

/// 面板本体。持有查询框、计数标签与结果列表，暴露两个回调；它不认识
/// StorageBridge，也不认识窗口。
final class SearchPanel: NSObject, NSTableViewDataSource, NSTableViewDelegate,
    NSSearchFieldDelegate {
    /// 面板的容器是 `NSBox` 而不是裸 `NSView`：后者是透明的，查询框会像浮在
    /// 侧栏中间，与上面的大纲之间也没有边界。`fillColor` 收的是动态
    /// `NSColor`，所以深浅色切换时它自己重绘——写死一个 layer 背景色不会。
    /// 这条是真实窗口截图抓出来的，全部自动化断言都绿。
    let view: NSView = {
        let box = NSBox()
        box.boxType = .custom
        box.borderWidth = 0.0
        box.titlePosition = .noTitle
        box.contentViewMargins = .zero
        box.fillColor = .controlBackgroundColor
        return box
    }()

    private let separator: NSBox = {
        let box = NSBox()
        box.boxType = .separator
        box.translatesAutoresizingMaskIntoConstraints = false
        return box
    }()

    private let field = NSSearchField()
    private let countLabel = NSTextField(labelWithString: "")
    private let scrollView = NSScrollView()
    private let tableView = NSTableView()
    private var rows: [SearchResultRow] = []

    /// 查询框里的字变了。防抖交给调用方——这里每敲一个字符都发一次。
    var onQueryChange: ((String) -> Void)?
    /// 点了某一行。程序化恢复选中时不触发。
    var onSelect: ((NativeSearchMatch) -> Void)?
    private var restoringSelection = false

    override init() {
        super.init()
        field.placeholderString = "搜索"
        field.delegate = self
        field.translatesAutoresizingMaskIntoConstraints = false
        field.sendsWholeSearchString = false
        field.sendsSearchStringImmediately = true

        countLabel.font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
        countLabel.textColor = .secondaryLabelColor
        countLabel.translatesAutoresizingMaskIntoConstraints = false

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("search"))
        column.resizingMask = .autoresizingMask
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.rowSizeStyle = .default
        tableView.style = .plain
        tableView.backgroundColor = .controlBackgroundColor
        tableView.dataSource = self
        tableView.delegate = self
        tableView.setAccessibilityLabel("搜索结果")
        // 单列表格：列宽必须跟着表走，否则 cell 再怎么约束也被一个默认宽度的
        // 列框住。
        tableView.columnAutoresizingStyle = .firstColumnOnlyAutoresizingStyle

        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = true
        scrollView.backgroundColor = .controlBackgroundColor
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        view.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(separator)
        view.addSubview(field)
        view.addSubview(countLabel)
        view.addSubview(scrollView)
        NSLayoutConstraint.activate([
            separator.topAnchor.constraint(equalTo: view.topAnchor),
            separator.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            field.topAnchor.constraint(equalTo: separator.bottomAnchor, constant: 6.0),
            field.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 6.0),
            field.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -6.0),
            countLabel.topAnchor.constraint(equalTo: field.bottomAnchor, constant: 4.0),
            countLabel.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 8.0),
            countLabel.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -6.0),
            scrollView.topAnchor.constraint(equalTo: countLabel.bottomAnchor, constant: 4.0),
            scrollView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
    }

    /// 面板要不要接受键盘焦点由窗口决定；这里只把查询框交出去。
    var focusTarget: NSView { field }

    var query: String { field.stringValue }

    func setQuery(_ query: String) {
        field.stringValue = query
    }

    /// 用新一版的结果整体重建。
    ///
    /// **没有展开状态要恢复，也没有跨刷新的身份要维持**——每换一次查询这一列
    /// 就整体换掉，条目的身份就是「第几个匹配」。这正是它与大纲面板要分开写
    /// 的那一半理由。
    /// `query` 是这一批结果对应的查询。**显式传进来**，不从查询框里读：
    /// 结果是照某一份查询算出来的，而查询框里的字随时可能已经是下一个了。
    func reload(rows: [SearchResultRow], query: String) {
        self.rows = rows
        tableView.reloadData()
        countLabel.stringValue = rows.isEmpty
            ? (query.isEmpty ? "" : "没有匹配")
            : "\(rows.count) 处匹配"
    }

    /// 把「当前命中」那一行选中，不触发导航回调。
    ///
    /// 当前命中由选区决定（Rust 侧那份定义），所以这里收的是选区，不是一个
    /// 面板自己维护的下标——存下标就会有第二个可以对不上的答案。
    func highlightRow(matching selection: NSRange) {
        restoringSelection = true
        defer { restoringSelection = false }
        guard let row = rows.firstIndex(where: { $0.match.range == selection }) else {
            tableView.deselectAll(nil)
            return
        }
        tableView.selectRowIndexes([row], byExtendingSelection: false)
        tableView.scrollRowToVisible(row)
    }

    // MARK: - NSSearchFieldDelegate

    func controlTextDidChange(_ notification: Notification) {
        onQueryChange?(field.stringValue)
    }

    // MARK: - NSTableViewDataSource / Delegate

    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

    func tableView(
        _ tableView: NSTableView,
        viewFor tableColumn: NSTableColumn?,
        row: Int
    ) -> NSView? {
        guard rows.indices.contains(row) else { return nil }
        let identifier = NSUserInterfaceItemIdentifier("search-cell")
        let cell: NSTableCellView
        if let reused = tableView.makeView(withIdentifier: identifier, owner: self)
            as? NSTableCellView {
            cell = reused
        } else {
            // 裸 `NSTextField` 当 cell 用时它的宽度不跟着列走，长一点的上下文
            // 会被**直接裁断**——连省略号都看不见。包一层并把左右钉在 cell 上，
            // 截断才落到 `byTruncatingTail` 上。同样是截图抓出来的。
            cell = NSTableCellView()
            cell.identifier = identifier
            let field = NSTextField(labelWithString: "")
            field.lineBreakMode = .byTruncatingTail
            field.translatesAutoresizingMaskIntoConstraints = false
            cell.addSubview(field)
            cell.textField = field
            NSLayoutConstraint.activate([
                field.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 8.0),
                field.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -8.0),
                field.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            ])
        }
        cell.textField?.stringValue = rows[row].label
        cell.toolTip = rows[row].label
        return cell
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard !restoringSelection else { return }
        let row = tableView.selectedRow
        guard rows.indices.contains(row) else { return }
        onSelect?(rows[row].match)
    }

    // MARK: - self-check 入口
    //
    // 「面板的条数与 FFI 一致」是自证的（面板本来就是照那个数组画的），
    // 所以这里只交出 NSTableView 眼里的行，断言写在 SelfChecks.swift。

    var rowCountForSelfCheck: Int { tableView.numberOfRows }

    func rowForSelfCheck(_ row: Int) -> SearchResultRow? {
        rows.indices.contains(row) ? rows[row] : nil
    }

    func clickRowForSelfCheck(_ row: Int) {
        tableView.deselectAll(nil)
        tableView.selectRowIndexes([row], byExtendingSelection: false)
    }

    var selectedRowForSelfCheck: Int { tableView.selectedRow }

    var countTextForSelfCheck: String { countLabel.stringValue }
}
