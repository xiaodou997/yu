import AppKit
import QuartzCore

enum NativeImageScalePreset: Equatable {
    case percent(Int)
    case original
    case fitColumn
}

struct NativeImageInteractionState {
    let revision: UInt64
    let sourceRange: NSRange
    let documentBounds: NSRect
    let sourceText: String
    let properties: NativeImageProperties
    let displayedDestination: String
    let hasIntrinsicSize: Bool
}

/// Lightweight image inspector rendered above the Metal surface. The surface
/// remains the only document renderer; this native view is transient chrome
/// for one explicitly selected image.
final class ImageInteractionOverlay: NSView, NSTextFieldDelegate {
    var onCommit: ((String, String) -> Void)?
    var onReplace: (() -> Void)?
    var onScale: ((NativeImageScalePreset) -> Void)?
    var onMore: ((NSView) -> Void)?

    private let selectionLayer = CAShapeLayer()
    private let panel = NSVisualEffectView()
    private let sourceLabel = NSTextField(labelWithString: "")
    private let alternative = NSTextField()
    private let destination = NSTextField()
    private let replaceButton = NSButton()
    private let sizeButton = NSButton()
    private let moreButton = NSButton()
    private var state: NativeImageInteractionState?
    private var imageRect = NSRect.zero
    private var updatingFields = false

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.masksToBounds = true
        selectionLayer.fillColor = NSColor.clear.cgColor
        selectionLayer.lineWidth = 2
        layer?.addSublayer(selectionLayer)

        panel.material = .popover
        panel.blendingMode = .withinWindow
        panel.state = .active
        panel.wantsLayer = true
        panel.layer?.cornerRadius = 9
        panel.layer?.borderWidth = 0.5

        sourceLabel.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        sourceLabel.textColor = .secondaryLabelColor
        sourceLabel.cell?.lineBreakMode = .byTruncatingMiddle
        sourceLabel.isSelectable = true
        sourceLabel.setAccessibilityLabel(L10n.tr("Image Source"))

        alternative.placeholderString = L10n.tr("Alternative Text")
        alternative.identifier = NSUserInterfaceItemIdentifier("yu-image-inline-alternative")
        alternative.setAccessibilityLabel(L10n.tr("Alternative Text"))
        destination.placeholderString = L10n.tr("Image Address")
        destination.identifier = NSUserInterfaceItemIdentifier("yu-image-inline-destination")
        destination.setAccessibilityLabel(L10n.tr("Image Address"))
        for field in [alternative, destination] {
            field.cell?.usesSingleLineMode = true
            field.cell?.isScrollable = true
            field.delegate = self
            field.target = self
            field.action = #selector(commitFields(_:))
        }

        replaceButton.image = NSImage(systemSymbolName: "folder", accessibilityDescription: L10n.tr("Replace Image…"))
        replaceButton.bezelStyle = .texturedRounded
        replaceButton.toolTip = L10n.tr("Replace Image…")
        replaceButton.setAccessibilityLabel(L10n.tr("Replace Image…"))
        replaceButton.target = self
        replaceButton.action = #selector(replaceImage(_:))

        sizeButton.title = L10n.tr("Image Size")
        sizeButton.bezelStyle = .texturedRounded
        sizeButton.toolTip = L10n.tr("Image Size")
        sizeButton.target = self
        sizeButton.action = #selector(showSizeMenu(_:))

        moreButton.image = NSImage(systemSymbolName: "ellipsis", accessibilityDescription: L10n.tr("More Image Actions"))
        moreButton.bezelStyle = .texturedRounded
        moreButton.toolTip = L10n.tr("More Image Actions")
        moreButton.setAccessibilityLabel(L10n.tr("More Image Actions"))
        moreButton.target = self
        moreButton.action = #selector(showMoreMenu(_:))

        let row = NSStackView(views: [alternative, destination, replaceButton, sizeButton, moreButton])
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = 6
        alternative.widthAnchor.constraint(equalToConstant: 145).isActive = true
        destination.widthAnchor.constraint(greaterThanOrEqualToConstant: 180).isActive = true
        replaceButton.widthAnchor.constraint(equalToConstant: 30).isActive = true
        sizeButton.widthAnchor.constraint(equalToConstant: 92).isActive = true
        moreButton.widthAnchor.constraint(equalToConstant: 30).isActive = true

        let stack = NSStackView(views: [sourceLabel, row])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 5
        stack.translatesAutoresizingMaskIntoConstraints = false
        panel.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: panel.leadingAnchor, constant: 10),
            stack.trailingAnchor.constraint(equalTo: panel.trailingAnchor, constant: -10),
            stack.topAnchor.constraint(equalTo: panel.topAnchor, constant: 8),
            stack.bottomAnchor.constraint(equalTo: panel.bottomAnchor, constant: -8),
            sourceLabel.widthAnchor.constraint(equalTo: stack.widthAnchor),
            row.widthAnchor.constraint(equalTo: stack.widthAnchor),
        ])
        addSubview(panel)
        isHidden = true
        setAccessibilityElement(false)
        updateAppearance()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        updateAppearance()
    }

    private func updateAppearance() {
        selectionLayer.strokeColor = NSColor.controlAccentColor.withAlphaComponent(0.85).cgColor
        panel.layer?.borderColor = NSColor.separatorColor.withAlphaComponent(0.7).cgColor
    }

    /// Only the inspector card owns pointer input. The selection outline stays
    /// click-through so a second click still reaches DocumentTextView.
    override func hitTest(_ point: NSPoint) -> NSView? {
        guard !isHidden, panel.frame.contains(point) else { return nil }
        return panel.hitTest(convert(point, to: panel))
    }

    func present(_ state: NativeImageInteractionState, imageRect: NSRect) {
        let sameIdentity = self.state?.revision == state.revision
            && self.state?.sourceRange == state.sourceRange
        self.state = state
        self.imageRect = imageRect
        sourceLabel.stringValue = state.sourceText
        sourceLabel.toolTip = state.sourceText
        // Scrolling and viewport relayout repeatedly repositions this overlay.
        // Do not clobber text that is currently being edited unless the
        // document/image identity actually changed.
        if !sameIdentity {
            updatingFields = true
            alternative.stringValue = state.properties.alternative
            destination.stringValue = state.displayedDestination
            updatingFields = false
        }
        isHidden = false
        updateFrames()
    }

    func updateImageRect(_ rect: NSRect) {
        imageRect = rect
        guard !isHidden else { return }
        updateFrames()
    }

    func dismiss() {
        state = nil
        isHidden = true
        selectionLayer.path = nil
    }

    override func layout() {
        super.layout()
        if !isHidden { updateFrames() }
    }

    private func updateFrames() {
        guard bounds.width > 0, bounds.height > 0 else { return }
        let outlined = imageRect.insetBy(dx: -2, dy: -2)
        selectionLayer.path = CGPath(roundedRect: outlined, cornerWidth: 7, cornerHeight: 7, transform: nil)

        let availableWidth = max(320, bounds.width - 24)
        let width = min(720, availableWidth)
        let height: CGFloat = 74
        var x = imageRect.midX - width / 2
        x = min(max(12, x), max(12, bounds.width - width - 12))
        var y = imageRect.maxY + 8
        if y + height > bounds.maxY - 8 {
            y = imageRect.minY - height - 8
        }
        y = min(max(8, y), max(8, bounds.maxY - height - 8))
        panel.frame = NSRect(x: x, y: y, width: width, height: height)
    }

    func controlTextDidEndEditing(_ notification: Notification) {
        commitIfNeeded()
    }

    @objc private func commitFields(_ sender: Any?) {
        window?.makeFirstResponder(self)
        commitIfNeeded()
    }

    private func commitIfNeeded() {
        guard !updatingFields, let state else { return }
        guard alternative.stringValue != state.properties.alternative
                || destination.stringValue != state.displayedDestination else { return }
        onCommit?(alternative.stringValue, destination.stringValue)
    }

    @objc private func replaceImage(_ sender: Any?) {
        onReplace?()
    }

    @objc private func showSizeMenu(_ sender: NSButton) {
        let menu = NSMenu()
        for percent in [25, 33, 50, 67, 80, 100, 150, 200] {
            let item = NSMenuItem(title: "\(percent)%", action: #selector(selectScale(_:)), keyEquivalent: "")
            item.target = self
            item.tag = percent
            item.isEnabled = state?.hasIntrinsicSize == true
            menu.addItem(item)
        }
        menu.addItem(.separator())
        let original = NSMenuItem(title: L10n.tr("Original Size"), action: #selector(selectScale(_:)), keyEquivalent: "")
        original.target = self
        original.tag = -1
        menu.addItem(original)
        let fit = NSMenuItem(title: L10n.tr("Fit to Column"), action: #selector(selectScale(_:)), keyEquivalent: "")
        fit.target = self
        fit.tag = -2
        menu.addItem(fit)
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: sender.bounds.height + 2), in: sender)
    }

    @objc private func selectScale(_ sender: NSMenuItem) {
        switch sender.tag {
        case -1: onScale?(.original)
        case -2: onScale?(.fitColumn)
        default: onScale?(.percent(sender.tag))
        }
    }

    @objc private func showMoreMenu(_ sender: NSButton) {
        onMore?(sender)
    }
}
