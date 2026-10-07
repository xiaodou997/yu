import AppKit

/// Native, click-through chrome for visible fenced code blocks. The retained
/// Rust surface remains the document renderer; only the copy buttons accept hits.
final class CodeBlockControlsOverlay: NSView {
    struct Item {
        let control: NativeCodeBlockControl
        let language: String
        let blockFrame: NSRect
        let copyFrame: NSRect
    }

    var onCopy: ((NativeCodeBlockControl) -> Bool)?

    private final class Entry {
        let label = NSTextField(labelWithString: "")
        let button = NSButton()
        var control: NativeCodeBlockControl
        var feedbackWorkItem: DispatchWorkItem?

        init(control: NativeCodeBlockControl) {
            self.control = control
        }

        deinit {
            feedbackWorkItem?.cancel()
        }
    }

    private var entries: [UInt64: Entry] = [:]

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.masksToBounds = true
        setAccessibilityElement(false)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        for entry in entries.values where !entry.button.isHidden && entry.button.frame.contains(point) {
            let local = convert(point, to: entry.button)
            return entry.button.hitTest(local)
        }
        return nil
    }

    func dismiss() {
        for entry in entries.values {
            entry.feedbackWorkItem?.cancel()
            entry.label.removeFromSuperview()
            entry.button.removeFromSuperview()
        }
        entries.removeAll(keepingCapacity: true)
    }

    func present(_ items: [Item]) {
        let live = Set(items.map { $0.control.blockIndex })
        let stale = entries.keys.filter { !live.contains($0) }
        for key in stale {
            guard let entry = entries.removeValue(forKey: key) else { continue }
            entry.feedbackWorkItem?.cancel()
            entry.label.removeFromSuperview()
            entry.button.removeFromSuperview()
        }

        for item in items {
            let entry = entries[item.control.blockIndex] ?? makeEntry(control: item.control)
            entry.control = item.control
            entry.label.stringValue = item.language
            entry.label.toolTip = item.language
            entry.label.frame = NSRect(
                x: item.blockFrame.minX + 12,
                y: item.copyFrame.minY,
                width: max(0, item.copyFrame.minX - item.blockFrame.minX - 20),
                height: item.copyFrame.height
            )
            entry.button.frame = item.copyFrame
            entry.button.isHidden = !bounds.intersects(item.copyFrame)
            entry.label.isHidden = !bounds.intersects(item.blockFrame)
        }
    }

    private func makeEntry(control: NativeCodeBlockControl) -> Entry {
        let entry = Entry(control: control)
        entry.label.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .medium)
        entry.label.textColor = .secondaryLabelColor
        entry.label.cell?.lineBreakMode = .byTruncatingTail
        entry.label.isSelectable = false
        entry.label.setAccessibilityElement(false)

        entry.button.bezelStyle = .texturedRounded
        entry.button.isBordered = false
        entry.button.imagePosition = .imageOnly
        entry.button.image = NSImage(
            systemSymbolName: "doc.on.doc",
            accessibilityDescription: L10n.tr("Copy")
        )
        entry.button.toolTip = L10n.tr("Copy")
        entry.button.setAccessibilityLabel(L10n.tr("Copy"))
        entry.button.target = self
        entry.button.action = #selector(copyCode(_:))
        entry.button.alphaValue = 0.72

        addSubview(entry.label)
        addSubview(entry.button)
        entries[control.blockIndex] = entry
        return entry
    }

    @objc private func copyCode(_ sender: NSButton) {
        guard let entry = entries.values.first(where: { $0.button === sender }),
              onCopy?(entry.control) == true else { return }
        entry.feedbackWorkItem?.cancel()
        entry.button.image = NSImage(
            systemSymbolName: "checkmark",
            accessibilityDescription: L10n.tr("Copy")
        )
        entry.button.alphaValue = 1.0
        let revision = entry.control.revision
        let blockIndex = entry.control.blockIndex
        let work = DispatchWorkItem { [weak self, weak button = entry.button] in
            guard let self, let button,
                  let current = self.entries[blockIndex],
                  current.button === button,
                  current.control.revision == revision else { return }
            button.image = NSImage(
                systemSymbolName: "doc.on.doc",
                accessibilityDescription: L10n.tr("Copy")
            )
            button.alphaValue = 0.72
        }
        entry.feedbackWorkItem = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.2, execute: work)
    }
}
