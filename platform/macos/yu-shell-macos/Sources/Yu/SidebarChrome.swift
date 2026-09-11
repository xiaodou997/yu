import AppKit

/// Shared visual chrome for the document sidebar.  The outline/search panels
/// remain responsible for their data and navigation; this file only owns the
/// AppKit presentation primitives that make the sidebar feel native.
final class YuSidebarHeaderView: NSView {
    private let logoView = NSImageView()
    private let titleLabel = NSTextField(labelWithString: "Yu")
    private let subtitleLabel = NSTextField(labelWithString: "")

    init(documentName: String) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        logoView.imageScaling = .scaleProportionallyUpOrDown
        logoView.image = Self.loadLogo()
        logoView.translatesAutoresizingMaskIntoConstraints = false
        logoView.setAccessibilityElement(true)
        logoView.setAccessibilityLabel("Yu")

        titleLabel.font = NSFont.systemFont(ofSize: 14.0, weight: .semibold)
        titleLabel.textColor = .labelColor
        titleLabel.translatesAutoresizingMaskIntoConstraints = false

        subtitleLabel.stringValue = documentName
        subtitleLabel.font = NSFont.systemFont(ofSize: 11.0)
        subtitleLabel.textColor = .secondaryLabelColor
        subtitleLabel.lineBreakMode = .byTruncatingMiddle
        subtitleLabel.translatesAutoresizingMaskIntoConstraints = false

        addSubview(logoView)
        addSubview(titleLabel)
        addSubview(subtitleLabel)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 56.0),
            logoView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12.0),
            logoView.centerYAnchor.constraint(equalTo: centerYAnchor),
            logoView.widthAnchor.constraint(equalToConstant: 24.0),
            logoView.heightAnchor.constraint(equalToConstant: 24.0),
            titleLabel.leadingAnchor.constraint(equalTo: logoView.trailingAnchor, constant: 9.0),
            titleLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10.0),
            titleLabel.topAnchor.constraint(equalTo: topAnchor, constant: 10.0),
            subtitleLabel.leadingAnchor.constraint(equalTo: titleLabel.leadingAnchor),
            subtitleLabel.trailingAnchor.constraint(equalTo: titleLabel.trailingAnchor),
            subtitleLabel.topAnchor.constraint(equalTo: titleLabel.bottomAnchor, constant: 2.0),
        ])

        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        setAccessibilityLabel("文档侧栏")
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    private static func loadLogo() -> NSImage? {
        guard let path = Bundle.main.path(forResource: "Yu", ofType: "png") else {
            return nil
        }
        return NSImage(contentsOfFile: path)
    }
}

/// A native-looking outline row with a quiet rounded selection and hover
/// state.  The row does not know anything about Markdown semantics.
final class YuSidebarRowView: NSTableRowView {
    private var tracking: NSTrackingArea?
    private var isHovered = false

    override func updateTrackingAreas() {
        if let tracking { removeTrackingArea(tracking) }
        let next = NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self,
            userInfo: nil
        )
        tracking = next
        addTrackingArea(next)
        super.updateTrackingAreas()
    }

    override func mouseEntered(with event: NSEvent) {
        isHovered = true
        needsDisplay = true
        super.mouseEntered(with: event)
    }

    override func mouseExited(with event: NSEvent) {
        isHovered = false
        needsDisplay = true
        super.mouseExited(with: event)
    }

    override func drawBackground(in dirtyRect: NSRect) {
        guard isHovered, !isSelected else { return }
        NSColor.quaternaryLabelColor.withAlphaComponent(0.12).setFill()
        let rect = bounds.insetBy(dx: 6.0, dy: 2.0)
        NSBezierPath(roundedRect: rect, xRadius: 6.0, yRadius: 6.0).fill()
    }

    override func drawSelection(in dirtyRect: NSRect) {
        guard isSelected else { return }
        let color = isEmphasized
            ? NSColor.selectedContentBackgroundColor
            : NSColor.unemphasizedSelectedContentBackgroundColor
        color.withAlphaComponent(0.88).setFill()
        let rect = bounds.insetBy(dx: 6.0, dy: 2.0)
        NSBezierPath(roundedRect: rect, xRadius: 6.0, yRadius: 6.0).fill()
    }
}

final class YuStatusBarView: NSVisualEffectView {
    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        material = .underWindowBackground
        blendingMode = .withinWindow
        state = .active
        translatesAutoresizingMaskIntoConstraints = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
}
