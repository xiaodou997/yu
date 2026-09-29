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
    private let autosave = NSButton(checkboxWithTitle: L10n.tr("Automatically save named documents"), target: nil, action: nil)
    private let spelling = NSButton(checkboxWithTitle: L10n.tr("System spelling check"), target: nil, action: nil)
    private let focusMode = NSButton(checkboxWithTitle: L10n.tr("Focus Mode (highlight current paragraph)"), target: nil, action: nil)
    private let typewriter = NSButton(checkboxWithTitle: L10n.tr("Typewriter Mode (keep active line centered while typing)"), target: nil, action: nil)
    private var themeObserver: NSObjectProtocol?

    init() {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 500, height: 520),
            styleMask: [.titled, .closable], backing: .buffered, defer: false)
        super.init(window: window)
        window.title = L10n.tr("Yu Settings")
        window.identifier = NSUserInterfaceItemIdentifier("yu-settings")
        window.isReleasedWhenClosed = false
        window.isRestorable = false
        window.delegate = self
        theme.addItems(withTitles: [L10n.tr("Yu (Follow System)"), "Github", "Night"])
        fontSize.addItems(withTitles: fontSizes.map { "\(Int($0)) pt" })
        columnWidth.addItems(withTitles: columnWidths.map { $0 == 0 ? L10n.tr("Follow Theme") : "\(Int($0)) pt" })
        fontSize.setAccessibilityLabel(L10n.tr("Body Font Size"))
        columnWidth.setAccessibilityLabel(L10n.tr("Reading Column Width"))
        fontSize.identifier = NSUserInterfaceItemIdentifier("yu-body-font-size")
        columnWidth.identifier = NSUserInterfaceItemIdentifier("yu-reading-column-width")
        imagePolicy.addItems(withTitles: [L10n.tr("Copy to document assets directory"), L10n.tr("Reference original file (absolute path)")])
        imageDirectory.placeholderString = "assets"
        imageDirectory.setAccessibilityLabel(L10n.tr("Image Assets Directory"))
        imageDirectory.identifier = NSUserInterfaceItemIdentifier("yu-image-directory")
        theme.setAccessibilityLabel(L10n.tr("Body Theme"))
        imagePolicy.setAccessibilityLabel(L10n.tr("Image Import Method"))
        for control in [theme, fontSize, columnWidth, imagePolicy, imageDirectory, autosave, typewriter, focusMode, spelling] as [NSControl] {
            control.target = self
            control.action = #selector(applyChanges(_:))
        }
        let grid = NSGridView(views: [
            [NSTextField(labelWithString: L10n.tr("Body Theme")), theme],
            [NSTextField(labelWithString: L10n.tr("Body Font Size")), fontSize],
            [NSTextField(labelWithString: L10n.tr("Reading Column Width")), columnWidth],
            [NSTextField(labelWithString: L10n.tr("Image Import")), imagePolicy],
            [NSTextField(labelWithString: L10n.tr("Image Directory")), imageDirectory],
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
        let note = NSTextField(wrappingLabelWithString: L10n.tr("The image directory is relative to the Markdown file. Clipboard images are always saved as asset files; when referencing the original file, keep the original image in place."))
        note.textColor = .secondaryLabelColor
        let reset = NSButton(title: L10n.tr("Restore Defaults"), target: self, action: #selector(resetPreferences(_:)))
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
