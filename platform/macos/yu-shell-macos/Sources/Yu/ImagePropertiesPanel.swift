import AppKit
import ImageIO

/// A sheet edits a captured source identity. The bridge owns syntax and undo.
final class ImagePropertiesPanel: NSWindowController, NSTextFieldDelegate, NSWindowDelegate {
    private let properties: NativeImageProperties
    private let originalDisplayedDestination: String
    private let documentURL: URL
    private let applyChange: (NativeImageProperties) throws -> Void
    private let finished: () -> Void
    private let destination = NSTextField()
    private let alternative = NSTextField()
    private let width = NSTextField()
    private let height = NSTextField()
    private let lock = NSButton(checkboxWithTitle: "锁定纵横比", target: nil, action: nil)
    private let errorLabel = NSTextField(wrappingLabelWithString: "")
    private var ratio: Double?
    private var lastDimension: NSTextField?

    init(properties: NativeImageProperties, document: URL,
         apply: @escaping (NativeImageProperties) throws -> Void, finished: @escaping () -> Void) {
        self.properties = properties
        self.originalDisplayedDestination = Self.isLocal(properties.destination)
            ? (properties.destination.removingPercentEncoding ?? properties.destination) : properties.destination
        self.documentURL = document
        self.applyChange = apply
        self.finished = finished
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 520, height: 360),
            styleMask: [.titled, .closable], backing: .buffered, defer: false)
        super.init(window: panel)
        panel.title = "图片属性"
        panel.identifier = NSUserInterfaceItemIdentifier("yu-image-properties")
        panel.isReleasedWhenClosed = false
        panel.delegate = self
        destination.stringValue = originalDisplayedDestination
        alternative.stringValue = properties.alternative
        width.stringValue = properties.identity.width == 0 ? "" : String(properties.identity.width)
        height.stringValue = properties.identity.height == 0 ? "" : String(properties.identity.height)
        for (field, name, identifier) in [
            (destination, "图片地址", "yu-image-destination"), (alternative, "替代文字", "yu-image-alternative"),
            (width, "宽度", "yu-image-width"), (height, "高度", "yu-image-height")
        ] {
            field.cell?.usesSingleLineMode = true
            field.cell?.isScrollable = true
            field.setAccessibilityLabel(name)
            field.identifier = NSUserInterfaceItemIdentifier(identifier)
            field.delegate = self
        }
        width.placeholderString = "自动"
        height.placeholderString = "自动"
        ratio = Self.intrinsicRatio(path: originalDisplayedDestination, document: document)
        if properties.identity.width > 0 && properties.identity.height > 0 {
            ratio = Double(properties.identity.width) / Double(properties.identity.height)
        }
        lock.state = .on
        lock.target = self
        lock.action = #selector(lockChanged(_:))
        let grid = NSGridView(views: [
            [NSTextField(labelWithString: "图片地址"), destination],
            [NSTextField(labelWithString: "替代文字"), alternative],
            [NSTextField(labelWithString: "宽度（pt）"), width],
            [NSTextField(labelWithString: "高度（pt）"), height],
            [NSView(), lock],
        ])
        grid.rowSpacing = 12
        grid.columnSpacing = 14
        grid.column(at: 0).width = 90
        grid.column(at: 0).xPlacement = .trailing
        grid.column(at: 1).xPlacement = .fill
        grid.cell(atColumnIndex: 1, rowIndex: 4).xPlacement = .leading
        let note = NSTextField(wrappingLabelWithString: "尺寸留空时使用原始大小，显示时会适应正文宽度。本地路径相对于文档目录；调整尺寸不会修改原图文件。")
        note.textColor = .secondaryLabelColor
        errorLabel.textColor = .systemRed
        let cancel = NSButton(title: "取消", target: self, action: #selector(cancel(_:)))
        cancel.keyEquivalent = "\u{1b}"
        let apply = NSButton(title: "应用", target: self, action: #selector(apply(_:)))
        apply.keyEquivalent = "\r"
        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        cancel.setContentHuggingPriority(.required, for: .horizontal)
        apply.setContentHuggingPriority(.required, for: .horizontal)
        let buttons = NSStackView(views: [spacer, cancel, apply])
        buttons.orientation = .horizontal
        buttons.distribution = .fill
        buttons.spacing = 12
        let stack = NSStackView(views: [grid, note, errorLabel, buttons])
        stack.orientation = .vertical
        stack.alignment = .trailing
        stack.spacing = 14
        stack.translatesAutoresizingMaskIntoConstraints = false
        panel.contentView?.addSubview(stack)
        if let content = panel.contentView {
            NSLayoutConstraint.activate([
                stack.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 24),
                stack.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -24),
                stack.topAnchor.constraint(equalTo: content.topAnchor, constant: 22),
                grid.widthAnchor.constraint(equalTo: stack.widthAnchor),
                note.widthAnchor.constraint(equalTo: stack.widthAnchor),
                errorLabel.widthAnchor.constraint(equalTo: stack.widthAnchor),
                buttons.widthAnchor.constraint(equalTo: stack.widthAnchor),
            ])
        }
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    func present(on parent: NSWindow) {
        guard let window else { return }
        parent.beginSheet(window)
        window.makeFirstResponder(alternative)
    }

    private static func isLocal(_ value: String) -> Bool {
        !value.contains("://") && !value.hasPrefix("data:")
    }

    private static func intrinsicRatio(path: String, document: URL) -> Double? {
        // Read only local metadata; opening a properties sheet never performs
        // a network request or decodes a full bitmap.
        guard isLocal(path) else { return nil }
        let url = path.hasPrefix("/") ? URL(fileURLWithPath: path)
            : document.deletingLastPathComponent().appendingPathComponent(path)
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let values = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let w = values[kCGImagePropertyPixelWidth] as? NSNumber,
              let h = values[kCGImagePropertyPixelHeight] as? NSNumber,
              w.doubleValue > 0, h.doubleValue > 0 else { return nil }
        return w.doubleValue / h.doubleValue
    }

    func controlTextDidChange(_ notification: Notification) {
        guard let field = notification.object as? NSTextField else { return }
        errorLabel.stringValue = ""
        if field === width || field === height { lastDimension = field }
        if field === destination {
            ratio = Self.intrinsicRatio(path: destination.stringValue, document: documentURL)
        }
    }

    func controlTextDidEndEditing(_ notification: Notification) {
        guard let field = notification.object as? NSTextField, field === width || field === height else { return }
        synchronizeRatio(from: field)
    }

    @objc private func lockChanged(_ sender: Any?) {
        errorLabel.stringValue = ""
        if lock.state == .on { synchronizeRatio(from: lastDimension ?? width) }
    }

    private func synchronizeRatio(from field: NSTextField) {
        guard lock.state == .on, let value = UInt32(field.stringValue), value > 0, value <= 100_000 else { return }
        let other = field === width ? height : width
        if let ratio, ratio.isFinite, ratio > 0 {
            let matched = (field === width ? Double(value) / ratio : Double(value) * ratio).rounded()
            guard matched >= 1, matched <= 100_000 else {
                errorLabel.stringValue = "按纵横比计算的尺寸超出范围，请调整尺寸。"
                return
            }
            other.stringValue = String(UInt32(matched))
        } else {
            // One explicit dimension lets the renderer preserve the intrinsic
            // aspect ratio after an unavailable/remote resource finishes loading.
            other.stringValue = ""
        }
    }

    @objc private func apply(_ sender: Any?) {
        window?.makeFirstResponder(nil)
        if let lastDimension { synchronizeRatio(from: lastDimension) }
        guard errorLabel.stringValue.isEmpty else { return }
        func dimension(_ field: NSTextField) -> UInt32? {
            let value = field.stringValue.trimmingCharacters(in: .whitespaces)
            if value.isEmpty { return 0 }
            guard let number = UInt32(value), number > 0, number <= 100_000 else { return nil }
            return number
        }
        guard let w = dimension(width), let h = dimension(height), !destination.stringValue.isEmpty else {
            errorLabel.stringValue = "请填写图片地址；尺寸须为 1–100000 的整数或留空。"
            return
        }
        var updated = properties
        do {
            if destination.stringValue != originalDisplayedDestination {
                updated.destination = Self.isLocal(destination.stringValue)
                    ? try StorageBridge.imageURI(forLocalPath: destination.stringValue) : destination.stringValue
            }
        } catch { errorLabel.stringValue = "图片地址无效，请重新输入本地路径。"; return }
        updated.alternative = alternative.stringValue
        updated.identity.width = w
        updated.identity.height = h
        do { try applyChange(updated); dismiss() }
        catch BridgeError.operation(let status) {
            errorLabel.stringValue = status == StorageStatus.staleRevision
                ? "文档已变化，请取消后重新打开图片属性。"
                : "无法应用图片属性，请检查地址和尺寸后重试。"
        }
        catch { errorLabel.stringValue = "无法应用图片属性，请取消后重试。" }
    }

    @objc private func cancel(_ sender: Any?) { dismiss() }
    func windowShouldClose(_ sender: NSWindow) -> Bool { dismiss(); return false }
    private func dismiss() {
        if let window { window.sheetParent?.endSheet(window); window.orderOut(nil) }
        finished()
    }
}
