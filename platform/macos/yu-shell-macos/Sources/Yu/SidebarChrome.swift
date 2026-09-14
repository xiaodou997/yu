import AppKit

/// 全壳共享的视觉 token。设计数值取自仓内高保真稿
/// `prototypes/mac-markdown/styles.css`；**颜色一律走这里的动态变体**，
/// `NSColor(name:dynamicProvider:)` 按绘制时的 effectiveAppearance 解析，
/// 壳里不许再出现字面量颜色——layer 的 `backgroundColor = 某某.cgColor` 会把
/// 颜色快照成纯色，深浅色切换时侧栏/rail 留在浅色（dark self-check 压住这条）。
enum YuVisualTokens {
    // MARK: 尺寸（设计稿 pt 值）

    /// 左缘 rail 宽。稿 58pt。
    static let railWidth: CGFloat = 58
    /// rail 按钮 36×36、圆角 9（稿 .rail-btn）。
    static let railButtonSize: CGFloat = 36
    static let railButtonRadius: CGFloat = 9
    static let railStackTopInset: CGFloat = 18
    static let railStackSpacing: CGFloat = 12

    /// 侧栏内容宽（稿 265pt；rail 另占 58）。
    static let sidebarWidth: CGFloat = 265
    /// 大纲/搜索结果行高 30（稿 32 是文件树行，面板索引用 30 更紧凑）。
    static let sidebarRowHeight: CGFloat = 30
    /// 行选中/hover 的圆角，Finder 侧栏那种整行圆角填充。
    static let sidebarRowRadius: CGFloat = 7
    /// 每级缩进（稿 .children 20px 偏大，面板用 14）。
    static let sidebarIndent: CGFloat = 14
    /// 行填充相对整行的左右/上下内缩。
    static let sidebarRowInsetX: CGFloat = 4
    static let sidebarRowInsetY: CGFloat = 2

    static let sectionHeaderHeight: CGFloat = 26
    static let statusBarHeight: CGFloat = 30

    // MARK: 阅读列（对标 Typora 860 列）

    static let readingColumnMaxWidth: CGFloat = 860
    static let readingColumnMinGutter: CGFloat = 48
    static let readingColumnTopInset: CGFloat = 56

    // MARK: 字号

    static let sectionHeaderFontSize: CGFloat = 11
    static let sidebarItemFontSize: CGFloat = 13
    static let countFontSize: CGFloat = 11
    static let statusBarFontSize: CGFloat = 11

    // MARK: 颜色（明暗双变体）

    /// 主题蓝。浅色沿用文件原值（≈#1F6EDD），深色用系统 dark accent #0A84FF。
    static let accent = dual(
        NSColor(calibratedRed: 0.12, green: 0.43, blue: 0.82, alpha: 1),
        NSColor(calibratedRed: 0.04, green: 0.52, blue: 1.0, alpha: 1)
    )
    /// 选中行底色（稿 --blue-soft #dcecff / 深色 ≈#2E4A6B）。
    static let accentSoft = dual(
        NSColor(calibratedRed: 0.86, green: 0.93, blue: 1.0, alpha: 1),
        NSColor(calibratedRed: 0.18, green: 0.29, blue: 0.42, alpha: 1)
    )
    /// 文档画布（滚动视口底色）。深色 ≈#1E1E20。
    static let canvas = dual(
        NSColor(calibratedWhite: 0.975, alpha: 1),
        NSColor(calibratedWhite: 0.118, alpha: 1)
    )
    /// rail 底色（稿 --rail #e7ebf0 / 深色略深于 sidebar 材质）。
    static let railCanvas = dual(
        NSColor(calibratedRed: 0.906, green: 0.922, blue: 0.941, alpha: 1),
        NSColor(calibratedWhite: 0.16, alpha: 1)
    )
    /// hover 水洗：稿 #fff8，取 #FFFFFF66（白 40%）；深色白 10%。
    static let hoverWash = dual(
        NSColor(calibratedWhite: 1.0, alpha: 0.4),
        NSColor(calibratedWhite: 1.0, alpha: 0.1)
    )
    /// rail 选中态白底浮起（稿 .rail-btn.selected #fff / 深色 #3A3A3C）。
    static let railSelection = dual(
        NSColor(calibratedWhite: 1.0, alpha: 1),
        NSColor(calibratedWhite: 0.227, alpha: 1)
    )
    /// rail 选中浮起的投影（稿 0 2px 8px #31445a18；深色换黑投影才有存在感）。
    static let railSelectionShadow = dual(
        NSColor(calibratedRed: 0.192, green: 0.267, blue: 0.353, alpha: 0.09),
        NSColor(calibratedWhite: 0.0, alpha: 0.3)
    )
    /// 分隔线（稿 --line #dfe4ea / 深色白 14%）。
    static let divider = dual(
        NSColor(calibratedRed: 0.875, green: 0.894, blue: 0.918, alpha: 1),
        NSColor(calibratedWhite: 1.0, alpha: 0.14)
    )

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

/// 左缘 rail：文档 / 大纲 / 搜索三个模式切换按钮。选中态是纯视觉
///（`selection` 由窗口按两个面板的实际显隐推导后写进来），点击通过
/// `onSelect` 交回窗口走同一套显隐逻辑。
final class YuSidebarRailView: NSView {
    enum Mode {
        case documents
        case outline
        case search
    }

    var onSelect: ((Mode) -> Void)?
    private var buttons: [Mode: YuRailButton] = [:]

    /// 选中态：nil 表示无选中（大纲与搜索同时可见的叠加态）。
    var selection: Mode? = nil {
        didSet {
            for (mode, button) in buttons {
                button.isOn = mode == selection
            }
        }
    }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        translatesAutoresizingMaskIntoConstraints = false

        let stack = NSStackView()
        stack.orientation = .vertical
        stack.spacing = YuVisualTokens.railStackSpacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        let specs: [(Mode, String, String)] = [
            (.documents, "doc.on.doc", "文档"),
            (.outline, "list.bullet", "大纲"),
            (.search, "magnifyingglass", "搜索"),
        ]
        for (mode, symbol, label) in specs {
            let button = YuRailButton(symbol: symbol, label: label)
            button.target = self
            button.action = #selector(railButtonTapped(_:))
            buttons[mode] = button
            stack.addArrangedSubview(button)
            NSLayoutConstraint.activate([
                button.widthAnchor.constraint(
                    equalToConstant: YuVisualTokens.railButtonSize
                ),
                button.heightAnchor.constraint(
                    equalToConstant: YuVisualTokens.railButtonSize
                ),
            ])
        }
        addSubview(stack)
        NSLayoutConstraint.activate([
            widthAnchor.constraint(equalToConstant: YuVisualTokens.railWidth),
            stack.topAnchor.constraint(
                equalTo: topAnchor,
                constant: YuVisualTokens.railStackTopInset
            ),
            stack.centerXAnchor.constraint(equalTo: centerXAnchor),
        ])
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    @objc private func railButtonTapped(_ sender: YuRailButton) {
        guard let mode = buttons.first(where: { $0.value === sender })?.key else {
            return
        }
        onSelect?(mode)
    }

    /// 背景用 draw 而不是 layer.fillColor：`NSView.draw` 里动态 token 按
    /// effectiveAppearance 自动解析，layer 那条 `cgColor` 路会把颜色快照死。
    override func draw(_ dirtyRect: NSRect) {
        YuVisualTokens.railCanvas.setFill()
        NSBezierPath(rect: bounds).fill()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }
}

/// rail 上的单个模式按钮。`isBordered = false` 的 NSButton 只画图标，
/// 背景（hover 水洗 / 选中白底浮起 + 轻阴影）由这里自绘，圆角 9 对标
/// 稿 .rail-btn。
final class YuRailButton: NSButton {
    var isOn = false {
        didSet {
            updateTint()
            needsDisplay = true
            setAccessibilitySelected(isOn)
        }
    }
    private var isHovered = false {
        didSet { needsDisplay = true }
    }

    init(symbol: String, label: String) {
        super.init(frame: .zero)
        image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)?
            .withSymbolConfiguration(
                NSImage.SymbolConfiguration(pointSize: 18, weight: .regular)
            )
        imagePosition = .imageOnly
        imageScaling = .scaleProportionallyDown
        isBordered = false
        translatesAutoresizingMaskIntoConstraints = false
        setAccessibilityLabel(label)
        updateTint()
    }

    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    private func updateTint() {
        contentTintColor = isOn ? YuVisualTokens.accent : .secondaryLabelColor
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for area in trackingAreas where area.owner === self {
            removeTrackingArea(area)
        }
        addTrackingArea(NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect],
            owner: self,
            userInfo: nil
        ))
    }

    override func mouseEntered(with event: NSEvent) {
        isHovered = true
        super.mouseEntered(with: event)
    }

    override func mouseExited(with event: NSEvent) {
        isHovered = false
        super.mouseExited(with: event)
    }

    override func draw(_ dirtyRect: NSRect) {
        let radius = YuVisualTokens.railButtonRadius
        let path = NSBezierPath(
            roundedRect: bounds,
            xRadius: radius,
            yRadius: radius
        )
        if isOn {
            // 选中浮起：白底 + 轻阴影（blur 8、透明度很低），稿
            // .rail-btn.selected 的 box-shadow。NSShadow.set() 让这一次
            // fill 同时投出阴影。
            let shadow = NSShadow()
            shadow.shadowBlurRadius = 8
            shadow.shadowOffset = NSSize(width: 0, height: -1.5)
            shadow.shadowColor = YuVisualTokens.railSelectionShadow
            NSGraphicsContext.saveGraphicsState()
            shadow.set()
            YuVisualTokens.railSelection.setFill()
            path.fill()
            NSGraphicsContext.restoreGraphicsState()
        } else if isHovered {
            YuVisualTokens.hoverWash.setFill()
            path.fill()
        }
        super.draw(dirtyRect)
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }
}

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

        titleLabel.font = NSFont.systemFont(ofSize: 15.0, weight: .semibold)
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
            heightAnchor.constraint(equalToConstant: 72.0),
            logoView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 16.0),
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

/// 面板区头：「大纲」小字重标题 + 右侧条目计数。计数由面板在每次 reload
/// 时写进来，不在窗口另存一份。
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

final class YuStatusBarView: NSVisualEffectView {
    private let divider: NSBox = {
        let box = NSBox()
        box.boxType = .custom
        box.borderWidth = 0
        box.fillColor = YuVisualTokens.divider
        // NSBox 的 fillColor 收动态 NSColor，深浅色切换时自己重绘（layer
        // 背景色那条路会把 cgColor 快照死）。
        box.translatesAutoresizingMaskIntoConstraints = false
        return box
    }()

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        material = .underWindowBackground
        blendingMode = .withinWindow
        state = .active
        translatesAutoresizingMaskIntoConstraints = false
        // 顶部分隔线：1pt 细线压在状态栏上缘，颜色走 token。
        addSubview(divider)
        NSLayoutConstraint.activate([
            divider.topAnchor.constraint(equalTo: topAnchor),
            divider.leadingAnchor.constraint(equalTo: leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: trailingAnchor),
            divider.heightAnchor.constraint(equalToConstant: 1.0),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
}
