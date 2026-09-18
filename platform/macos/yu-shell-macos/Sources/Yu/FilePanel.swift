import AppKit

/// Native file navigation. Children are loaded only when their folder expands.
final class FilePanel: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate {
    final class Item {
        let url: URL
        let directory: Bool
        var children: [Item]?
        init(_ url: URL) {
            self.url = url
            directory = (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true
        }
    }
    let view = NSScrollView()
    private let outline = NSOutlineView()
    private var root: Item
    var onOpen: ((URL) -> Void)?

    init(directory: URL) {
        root = Item(directory)
        super.init()
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("file"))
        outline.addTableColumn(column)
        outline.outlineTableColumn = column
        outline.headerView = nil
        outline.style = .sourceList
        outline.rowSizeStyle = .custom
        outline.rowHeight = YuVisualTokens.sidebarRowHeight
        outline.indentationPerLevel = 14
        outline.columnAutoresizingStyle = .firstColumnOnlyAutoresizingStyle
        outline.dataSource = self
        outline.delegate = self
        outline.target = self
        outline.doubleAction = #selector(openSelected(_:))
        outline.setAccessibilityLabel("文件")
        let menu = NSMenu()
        menu.addItem(withTitle: "打开", action: #selector(openSelected(_:)), keyEquivalent: "").target = self
        menu.addItem(withTitle: "在 Finder 中显示", action: #selector(revealSelected(_:)), keyEquivalent: "").target = self
        outline.menu = menu
        view.documentView = outline
        view.hasVerticalScroller = true
        view.autohidesScrollers = true
        view.drawsBackground = false
        view.translatesAutoresizingMaskIntoConstraints = false
    }
    func setDirectory(_ url: URL) {
        root = Item(url)
        outline.reloadData()
    }
    private func children(_ item: Item) -> [Item] {
        if let cached = item.children { return cached }
        let urls = (try? FileManager.default.contentsOfDirectory(at: item.url,
            includingPropertiesForKeys: [.isDirectoryKey], options: [.skipsHiddenFiles])) ?? []
        let result = urls.map(Item.init).filter {
            $0.directory || ["md", "markdown", "mdown", "txt"].contains($0.url.pathExtension.lowercased())
        }.sorted {
            if $0.directory != $1.directory { return $0.directory }
            return $0.url.lastPathComponent.localizedStandardCompare($1.url.lastPathComponent) == .orderedAscending
        }
        item.children = result
        return result
    }
    func outlineView(_ view: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        children(item as? Item ?? root).count
    }
    func outlineView(_ view: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        children(item as? Item ?? root)[index]
    }
    func outlineView(_ view: NSOutlineView, isItemExpandable item: Any) -> Bool {
        (item as? Item)?.directory == true
    }
    func outlineView(_ view: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any) -> NSView? {
        guard let item = item as? Item else { return nil }
        let cell = NSTableCellView()
        let label = NSTextField(labelWithString: item.url.lastPathComponent)
        label.font = .systemFont(ofSize: 13)
        label.lineBreakMode = .byTruncatingMiddle
        label.translatesAutoresizingMaskIntoConstraints = false
        let icon = NSImageView(image: NSImage(systemSymbolName: item.directory ? "folder" : "doc.text", accessibilityDescription: nil)!)
        icon.contentTintColor = .secondaryLabelColor
        icon.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(icon); cell.addSubview(label)
        cell.textField = label; cell.imageView = icon
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 2),
            icon.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 14), icon.heightAnchor.constraint(equalToConstant: 14),
            label.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 6),
            label.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -6),
            label.centerYAnchor.constraint(equalTo: cell.centerYAnchor)
        ])
        return cell
    }
    private var selected: Item? {
        outline.item(atRow: outline.clickedRow >= 0 ? outline.clickedRow : outline.selectedRow) as? Item
    }
    @objc private func openSelected(_ sender: Any?) {
        guard let item = selected else { return }
        if item.directory {
            if outline.isItemExpanded(item) { outline.collapseItem(item) } else { outline.expandItem(item) }
        } else { onOpen?(item.url) }
    }
    @objc private func revealSelected(_ sender: Any?) {
        guard let item = selected else { return }
        NSWorkspace.shared.activateFileViewerSelecting([item.url])
    }
}
