import AppKit

/// Product information and open-source support entry points.
final class NativeAboutWindowController: NSWindowController {
    static let repositoryURL = URL(string: "https://github.com/xiaodou997/yu")!
    static let issueURL = URL(string: "https://github.com/xiaodou997/yu/issues/new/choose")!

    init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 440, height: 370),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        super.init(window: window)
        window.title = L10n.tr("About Yu")
        window.identifier = NSUserInterfaceItemIdentifier("yu-about")
        window.isReleasedWhenClosed = false
        window.isRestorable = false

        let icon = NSImageView(image: NSApp.applicationIconImage)
        icon.imageScaling = .scaleProportionallyUpOrDown
        icon.translatesAutoresizingMaskIntoConstraints = false

        let name = NSTextField(labelWithString: "Yu")
        name.font = .systemFont(ofSize: 24, weight: .semibold)
        name.alignment = .center

        let tagline = NSTextField(labelWithString: L10n.tr("Visual Markdown Editor"))
        tagline.font = .systemFont(ofSize: 14, weight: .medium)
        tagline.textColor = .secondaryLabelColor
        tagline.alignment = .center

        let version = (Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String)
            ?? (Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String)
            ?? "—"
        let versionLabel = NSTextField(labelWithString: L10n.format("Version %@", version))
        versionLabel.textColor = .secondaryLabelColor
        versionLabel.alignment = .center

        let description = NSTextField(
            wrappingLabelWithString: L10n.tr(
                "Yu is open source software. If you run into a problem or have a feature suggestion, please report it on GitHub so it can be tracked and discussed."
            )
        )
        description.alignment = .center
        description.maximumNumberOfLines = 3

        let repository = NSButton(
            title: L10n.tr("GitHub Repository"),
            target: self,
            action: #selector(openRepository(_:))
        )
        repository.bezelStyle = .rounded

        let report = NSButton(
            title: L10n.tr("Report an Issue…"),
            target: self,
            action: #selector(reportIssue(_:))
        )
        report.bezelStyle = .rounded

        let buttons = NSStackView(views: [repository, report])
        buttons.orientation = .horizontal
        buttons.alignment = .centerY
        buttons.distribution = .fillEqually
        buttons.spacing = 12

        let license = NSTextField(labelWithString: L10n.tr("Open Source · Apache 2.0"))
        license.textColor = .tertiaryLabelColor
        license.alignment = .center

        let stack = NSStackView(views: [icon, name, tagline, versionLabel, description, buttons, license])
        stack.orientation = .vertical
        stack.alignment = .centerX
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false
        window.contentView?.addSubview(stack)

        NSLayoutConstraint.activate([
            icon.widthAnchor.constraint(equalToConstant: 80),
            icon.heightAnchor.constraint(equalToConstant: 80),
            stack.leadingAnchor.constraint(equalTo: window.contentView!.leadingAnchor, constant: 28),
            stack.trailingAnchor.constraint(equalTo: window.contentView!.trailingAnchor, constant: -28),
            stack.topAnchor.constraint(equalTo: window.contentView!.topAnchor, constant: 24),
            description.widthAnchor.constraint(equalTo: stack.widthAnchor),
            buttons.widthAnchor.constraint(equalTo: stack.widthAnchor, multiplier: 0.78),
        ])

        window.center()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    @objc private func openRepository(_ sender: Any?) {
        NSWorkspace.shared.open(Self.repositoryURL)
    }

    @objc private func reportIssue(_ sender: Any?) {
        NSWorkspace.shared.open(Self.issueURL)
    }
}
