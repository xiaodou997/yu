import AppKit

/// Native chrome metrics and dynamic colors from the shared resolved theme.
enum YuVisualTokens {
    static let sidebarWidth: CGFloat = CGFloat(NativeTheme.spec().sidebar_width)
    static let sidebarRowHeight: CGFloat = 24
    /// 行选中/hover 的圆角，Finder 侧栏那种整行圆角填充。
    static let sidebarRowRadius: CGFloat = 3
    static let sidebarIndent: CGFloat = 14
    /// 行填充相对整行的左右/上下内缩。
    static let sidebarRowInsetX: CGFloat = 4
    static let sidebarRowInsetY: CGFloat = 0

    static let sectionHeaderHeight: CGFloat = 26
    static let statusBarHeight: CGFloat = 22

    // MARK: 阅读列（对标 Typora 860 列）

    static let readingColumnMaxWidth: CGFloat = CGFloat(NativeTheme.spec().column_width)
    static let readingColumnMinGutter: CGFloat = CGFloat(NativeTheme.spec().gutter)
    static let readingColumnTopInset: CGFloat = CGFloat(NativeTheme.spec().top)

    // MARK: 字号

    static let sectionHeaderFontSize: CGFloat = 11
    static let sidebarItemFontSize: CGFloat = 13
    static let countFontSize: CGFloat = 11
    static let statusBarFontSize: CGFloat = 11

    // MARK: 颜色（明暗双变体）

    static let accent = NSColor.controlAccentColor
    static let accentSoft = dual(
        NSColor(calibratedRed: 0.86, green: 0.93, blue: 1.0, alpha: 1),
        NSColor(calibratedRed: 0.18, green: 0.29, blue: 0.42, alpha: 1)
    )
    static let canvas = NativeTheme.color(\.background)
    static let hoverWash = dual(
        NSColor(calibratedWhite: 1.0, alpha: 0.4),
        NSColor(calibratedWhite: 1.0, alpha: 0.1)
    )
    static let divider = NSColor.separatorColor

    private static func dual(
        _ light: @autoclosure @escaping () -> NSColor,
        _ dark: @autoclosure @escaping () -> NSColor
    ) -> NSColor {
        NSColor(name: nil, dynamicProvider: { appearance in
            appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
                ? dark()
                : light()
        })
    }

    /// self-check 专用：把一枚动态 token 在指定外观下解析成 1×1 像素后取回
    /// sRGB 亮度。生产路径不调用。dark self-check 用它压住「cgColor/layer 把
    /// 颜色写死成浅色」那一类回归：token 必须在 darkAqua 下真的变深。
    static func luminanceForSelfCheck(of color: NSColor, appearance: NSAppearance) -> CGFloat? {
        let size = NSSize(width: 1, height: 1)
        let image = NSImage(size: size)
        image.lockFocusFlipped(true)
        // performAsCurrentDrawingAppearance：动态 token 按这次绘制的外观解析。
        appearance.performAsCurrentDrawingAppearance {
            color.setFill()
            NSBezierPath(rect: NSRect(origin: .zero, size: size)).fill()
        }
        image.unlockFocus()
        guard let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff),
              let sampled = rep.colorAt(x: 0, y: 0)?
              .usingColorSpace(.sRGB) else {
            return nil
        }
        return (sampled.redComponent + sampled.greenComponent + sampled.blueComponent) / 3
    }
}

final class YuSidebarSectionHeader: NSView {
    private let titleLabel = NSTextField(labelWithString: "")
    private let countLabel = NSTextField(labelWithString: "")

    var count: Int = 0 {
        didSet { countLabel.stringValue = "\(count) 项" }
    }

    init(title: String) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        titleLabel.stringValue = title
        titleLabel.font = NSFont.systemFont(
            ofSize: YuVisualTokens.sectionHeaderFontSize,
            weight: .semibold
        )
        titleLabel.textColor = .secondaryLabelColor
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        titleLabel.setAccessibilityElement(false)

        countLabel.font = NSFont.systemFont(ofSize: YuVisualTokens.countFontSize)
        countLabel.textColor = .tertiaryLabelColor
        countLabel.alignment = .right
        countLabel.translatesAutoresizingMaskIntoConstraints = false
        countLabel.setAccessibilityElement(false)

        addSubview(titleLabel)
        addSubview(countLabel)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(
                equalToConstant: YuVisualTokens.sectionHeaderHeight
            ),
            titleLabel.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 16.0),
            titleLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
            countLabel.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12.0),
            countLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
            countLabel.leadingAnchor.constraint(
                greaterThanOrEqualTo: titleLabel.trailingAnchor,
                constant: 8.0
            ),
        ])

        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        setAccessibilityLabel(title)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
}

/// 大纲与搜索结果共用的行视图：圆角 7 整行填充，Finder 侧栏选中那种。
/// hover 走 hoverWash 水洗，选中走 accentSoft（不再自绘 18% alpha 的
/// accent 叠加），行不认识 Markdown 语义。
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

    private var fillRect: NSRect {
        bounds.insetBy(
            dx: YuVisualTokens.sidebarRowInsetX,
            dy: YuVisualTokens.sidebarRowInsetY
        )
    }

    override func drawBackground(in dirtyRect: NSRect) {
        guard isHovered, !isSelected else { return }
        YuVisualTokens.hoverWash.setFill()
        NSBezierPath(
            roundedRect: fillRect,
            xRadius: YuVisualTokens.sidebarRowRadius,
            yRadius: YuVisualTokens.sidebarRowRadius
        ).fill()
    }

    override func drawSelection(in dirtyRect: NSRect) {
        guard isSelected else { return }
        // 选中不随 key/non-key 换配方：面板在后台窗口里也保持 accentSoft，
        // 与 Finder 侧栏一致（窗口失焦只整体变暗，不由行自己调色）。
        YuVisualTokens.accentSoft.setFill()
        NSBezierPath(
            roundedRect: fillRect,
            xRadius: YuVisualTokens.sidebarRowRadius,
            yRadius: YuVisualTokens.sidebarRowRadius
        ).fill()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }
}

/// Opaque native surfaces keep theme colors independent of desktop wallpaper.
class NativeBackgroundView: NSView {
    private let color: NSColor
    init(color: NSColor) {
        self.color = color
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
        clipsToBounds = true
    }
    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    override var isOpaque: Bool { true }
    override func draw(_ dirtyRect: NSRect) {
        color.setFill()
        bounds.intersection(dirtyRect).fill()
    }
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }
}

/// Status text overlays the canvas without reserving document height. The
/// labels remain accessible, while pointer input reaches the editor beneath.
final class YuStatusBarView: NSView {
    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
    }
    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}
