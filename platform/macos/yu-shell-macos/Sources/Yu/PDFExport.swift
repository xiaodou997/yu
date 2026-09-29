import AppKit
import Foundation

/// PDF settings are independent of editor preferences and screen presentation.
/// The task/atomic publication controller is shared with HTML; legacy C symbol
/// names remain ABI-compatible and the explicit format lives in the config.
@MainActor
final class NativePDFExportOptions: NSObject {
    let view = NSStackView()
    private let paper = NSPopUpButton()
    private let orientation = NSPopUpButton()
    private let margin = NSTextField(string: "44")
    private let pageNumbers = NSButton(checkboxWithTitle: L10n.tr("Show page numbers"), target: nil, action: nil)
    private let baseLabel = NSTextField(labelWithString: L10n.tr("Image base directory: Not selected"))
    private var resourceBase: URL?

    init(untitled: Bool) {
        super.init()
        let preferences = UserDefaults.standard
        paper.addItems(withTitles: ["A4", "Letter"])
        paper.selectItem(withTitle: preferences.string(forKey: "Yu.exportPDF.paper") ?? "A4")
        orientation.addItems(withTitles: [L10n.tr("Portrait"), L10n.tr("Landscape")])
        orientation.selectItem(at: preferences.bool(forKey: "Yu.exportPDF.landscape") ? 1 : 0)
        if preferences.object(forKey: "Yu.exportPDF.margin") != nil {
            margin.doubleValue = min(144, max(18, preferences.double(forKey: "Yu.exportPDF.margin")))
        }
        pageNumbers.state = preferences.object(forKey: "Yu.exportPDF.pageNumbers") == nil || preferences.bool(forKey: "Yu.exportPDF.pageNumbers") ? .on : .off
        paper.setAccessibilityIdentifier("yu-pdf-paper")
        orientation.setAccessibilityIdentifier("yu-pdf-orientation")
        margin.setAccessibilityIdentifier("yu-pdf-margin")
        pageNumbers.setAccessibilityIdentifier("yu-pdf-page-numbers")
        margin.widthAnchor.constraint(equalToConstant: 70).isActive = true
        view.orientation = .vertical; view.alignment = .leading; view.spacing = 8
        view.addArrangedSubview(NSStackView(views: [NSTextField(labelWithString: L10n.tr("Paper:")), paper, orientation]))
        view.addArrangedSubview(NSStackView(views: [NSTextField(labelWithString: L10n.tr("Margins (18–144 pt):")), margin]))
        view.addArrangedSubview(pageNumbers)
        view.addArrangedSubview(NSTextField(labelWithString: L10n.tr("Uses a fixed light paper style and does not change the editor appearance.")))
        if untitled {
            view.addArrangedSubview(NSButton(title: L10n.tr("Choose image base directory…"), target: self, action: #selector(chooseBase)))
            view.addArrangedSubview(baseLabel)
        }
        view.frame = NSRect(x: 0, y: 0, width: 390, height: untitled ? 190 : 135)
    }
    @objc private func chooseBase() {
        let panel = NSOpenPanel()
        panel.title = L10n.tr("Choose the base directory for relative image paths")
        panel.canChooseDirectories = true; panel.canChooseFiles = false; panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url {
            resourceBase = url; baseLabel.stringValue = L10n.format("Image base directory: %@", url.lastPathComponent)
        }
    }
    func config(title: String, untitled: Bool) throws -> [String: Any] {
        guard let points = Double(margin.stringValue), points.isFinite, (18...144).contains(points) else {
            throw NSError(domain: "Yu.Export.PDF", code: 1, userInfo: [NSLocalizedDescriptionKey: L10n.tr("PDF margins must be between 18 and 144 pt.")])
        }
        let selectedPaper = paper.titleOfSelectedItem ?? "A4"
        let landscape = orientation.indexOfSelectedItem == 1
        let numbered = pageNumbers.state == .on
        let preferences = UserDefaults.standard
        preferences.set(selectedPaper, forKey: "Yu.exportPDF.paper")
        preferences.set(landscape, forKey: "Yu.exportPDF.landscape")
        preferences.set(points, forKey: "Yu.exportPDF.margin")
        preferences.set(numbered, forKey: "Yu.exportPDF.pageNumbers")
        var config: [String: Any] = ["exportFormat": "pdf", "title": title, "paper": selectedPaper,
            "landscape": landscape, "margin": points, "pageNumbers": numbered,
            "referenceDay": RenderCalendarContext.referenceDay(at: Date(), timeZone: .current) ?? 0,
            "untitled": untitled, "replaceExisting": true]
        if let resourceBase { config["resourceBase"] = resourceBase.path }
        return config
    }
}
