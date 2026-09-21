import AppKit

/// Standard AppKit controls; only implemented preferences appear here.
final class NativeSettingsWindowController: NSWindowController, NSWindowDelegate {
    var onChange: (() -> Void)?
    private let theme = NSPopUpButton()
    private let fontSize = NSPopUpButton()
    private let columnWidth = NSPopUpButton()
    private let fontSizes: [Double] = [12, 14, 16, 18, 20, 24, 28, 32]
    private let columnWidths: [Double] = [0, 480, 600, 760, 900, 1100]
    private let imagePolicy = NSPopUpButton()
    private let imageDirectory = NSTextField(string: "")
    private let autosave = NSButton(checkboxWithTitle: "自动保存已命名文档", target: nil, action: nil)
    private let spelling = NSButton(checkboxWithTitle: "系统拼写检查", target: nil, action: nil)
    private let focusMode = NSButton(checkboxWithTitle: "专注模式（突出当前段落）", target: nil, action: nil)
    private let typewriter = NSButton(checkboxWithTitle: "打字机模式（输入时保持活动行居中）", target: nil, action: nil)
    private var themeObserver: NSObjectProtocol?

    init() {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 500, height: 520),
            styleMask: [.titled, .closable], backing: .buffered, defer: false)
        super.init(window: window)
        window.title = "Yu 设置"
        window.identifier = NSUserInterfaceItemIdentifier("yu-settings")
        window.isReleasedWhenClosed = false
        window.isRestorable = false
        window.delegate = self
        theme.addItems(withTitles: ["Yu（跟随系统）", "Github", "Night"])
        fontSize.addItems(withTitles: fontSizes.map { "\(Int($0)) pt" })
        columnWidth.addItems(withTitles: columnWidths.map { $0 == 0 ? "随主题" : "\(Int($0)) pt" })
        fontSize.setAccessibilityLabel("正文字号")
        columnWidth.setAccessibilityLabel("正文行宽")
        fontSize.identifier = NSUserInterfaceItemIdentifier("yu-body-font-size")
        columnWidth.identifier = NSUserInterfaceItemIdentifier("yu-reading-column-width")
        imagePolicy.addItems(withTitles: ["复制到文档资源目录", "引用原文件（绝对路径）"])
        imageDirectory.placeholderString = "assets"
        imageDirectory.setAccessibilityLabel("图片资源目录")
        imageDirectory.identifier = NSUserInterfaceItemIdentifier("yu-image-directory")
        theme.setAccessibilityLabel("正文主题")
        imagePolicy.setAccessibilityLabel("图片导入方式")
        for control in [theme, fontSize, columnWidth, imagePolicy, imageDirectory, autosave, typewriter, focusMode, spelling] as [NSControl] {
            control.target = self
            control.action = #selector(applyChanges(_:))
        }
        let grid = NSGridView(views: [
            [NSTextField(labelWithString: "正文主题"), theme],
            [NSTextField(labelWithString: "正文字号"), fontSize],
            [NSTextField(labelWithString: "正文行宽"), columnWidth],
            [NSTextField(labelWithString: "图片导入"), imagePolicy],
            [NSTextField(labelWithString: "图片目录"), imageDirectory],
            [NSView(), autosave],
            [NSView(), typewriter],
            [NSView(), focusMode],
            [NSView(), spelling],
        ])
        grid.rowSpacing = 16
        grid.columnSpacing = 16
        grid.column(at: 0).width = 96
        grid.column(at: 0).xPlacement = .trailing
        grid.column(at: 1).xPlacement = .fill
        grid.cell(atColumnIndex: 1, rowIndex: 5).xPlacement = .leading
        grid.cell(atColumnIndex: 1, rowIndex: 6).xPlacement = .leading
        grid.cell(atColumnIndex: 1, rowIndex: 7).xPlacement = .leading
        grid.cell(atColumnIndex: 1, rowIndex: 8).xPlacement = .leading
        let note = NSTextField(wrappingLabelWithString: "图片目录相对于 Markdown 文件。剪贴板图片始终保存为资源文件；引用原文件时，请保留原图的位置。")
        note.textColor = .secondaryLabelColor
        let reset = NSButton(title: "恢复默认设置", target: self, action: #selector(resetPreferences(_:)))
        let stack = NSStackView(views: [grid, note, reset])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 20
        stack.translatesAutoresizingMaskIntoConstraints = false
        window.contentView?.addSubview(stack)
        if let content = window.contentView {
            NSLayoutConstraint.activate([
                stack.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 24),
                stack.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -24),
                stack.topAnchor.constraint(equalTo: content.topAnchor, constant: 24),
                grid.widthAnchor.constraint(equalTo: stack.widthAnchor),
                note.widthAnchor.constraint(equalTo: stack.widthAnchor),
            ])
        }
        themeObserver = NotificationCenter.default.addObserver(forName: NativeTheme.didChange, object: nil, queue: .main) { [weak self] _ in
            self?.loadValues()
        }
        loadValues()
        window.center()
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    deinit { if let themeObserver { NotificationCenter.default.removeObserver(themeObserver) } }

    func windowDidBecomeKey(_ notification: Notification) { loadValues() }

    private func loadValues() {
        theme.selectItem(at: NativeTheme.selection.rawValue)
        imagePolicy.selectItem(at: NativeWritingPreferences.shared.imagePolicy == .copy ? 0 : 1)
        imageDirectory.stringValue = NativeWritingPreferences.shared.imageDirectory
        fontSize.selectItem(at: fontSizes.firstIndex(of: NativeWritingPreferences.shared.fontSize) ?? 2)
        columnWidth.selectItem(at: columnWidths.firstIndex(of: NativeWritingPreferences.shared.columnWidth) ?? 0)
        spelling.state = NativeWritingPreferences.shared.spellingEnabled ? .on : .off
        focusMode.state = NativeWritingPreferences.shared.focusMode ? .on : .off
        typewriter.state = NativeWritingPreferences.shared.typewriterMode ? .on : .off
        autosave.state = NativeDocumentPersistence.autosaveEnabled ? .on : .off
    }

    @objc private func applyChanges(_ sender: Any?) {
        let selectedTheme = NativeTheme.Selection(rawValue: theme.indexOfSelectedItem) ?? .yu
        let selectedPolicy: NativeWritingPreferences.ImagePolicy = imagePolicy.indexOfSelectedItem == 0 ? .copy : .reference
        let automatic = autosave.state == .on
        do { try NativeWritingPreferences.shared.setImageDirectory(imageDirectory.stringValue) }
        catch { NSAlert(error: error).runModal(); loadValues(); return }
        NativeWritingPreferences.shared.spellingEnabled = spelling.state == .on
        NativeWritingPreferences.shared.focusMode = focusMode.state == .on
        NativeWritingPreferences.shared.typewriterMode = typewriter.state == .on
        NativeWritingPreferences.shared.fontSize = fontSizes[max(fontSize.indexOfSelectedItem, 0)]
        NativeWritingPreferences.shared.columnWidth = columnWidths[max(columnWidth.indexOfSelectedItem, 0)]
        NativeWritingPreferences.shared.imagePolicy = selectedPolicy
        NativeTheme.selection = selectedTheme
        NativeDocumentPersistence.autosaveEnabled = automatic
        publishChanges()
    }

    @objc private func resetPreferences(_ sender: Any?) {
        NativeWritingPreferences.shared.resetImagePreferences()
        NativeWritingPreferences.shared.resetReadingPreferences()
        NativeTheme.selection = .yu
        NativeDocumentPersistence.autosaveEnabled = true
        loadValues()
        publishChanges()
    }

    private func publishChanges() {
        NotificationCenter.default.post(name: NativeTheme.didChange, object: nil)
        NotificationCenter.default.post(name: NativeWritingPreferences.didChange, object: nil)
        onChange?()
    }
}
