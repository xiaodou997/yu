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
    private let pageNumbers = NSButton(checkboxWithTitle: "显示页码", target: nil, action: nil)
    private let baseLabel = NSTextField(labelWithString: "图片基准目录：未选择")
    private var resourceBase: URL?

    init(untitled: Bool) {
        super.init()
        let preferences = UserDefaults.standard
        paper.addItems(withTitles: ["A4", "Letter"])
        paper.selectItem(withTitle: preferences.string(forKey: "Yu.exportPDF.paper") ?? "A4")
        orientation.addItems(withTitles: ["纵向", "横向"])
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
        view.addArrangedSubview(NSStackView(views: [NSTextField(labelWithString: "纸张："), paper, orientation]))
        view.addArrangedSubview(NSStackView(views: [NSTextField(labelWithString: "页边距（18–144 pt）："), margin]))
        view.addArrangedSubview(pageNumbers)
        view.addArrangedSubview(NSTextField(labelWithString: "固定浅色纸张样式，不改变编辑器显示。"))
        if untitled {
            view.addArrangedSubview(NSButton(title: "选择图片基准目录…", target: self, action: #selector(chooseBase)))
            view.addArrangedSubview(baseLabel)
        }
        view.frame = NSRect(x: 0, y: 0, width: 390, height: untitled ? 190 : 135)
    }
    @objc private func chooseBase() {
        let panel = NSOpenPanel()
        panel.title = "选择相对图片路径的基准目录"
        panel.canChooseDirectories = true; panel.canChooseFiles = false; panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url {
            resourceBase = url; baseLabel.stringValue = "图片基准目录：\(url.lastPathComponent)"
        }
    }
    func config(title: String, untitled: Bool) throws -> [String: Any] {
        guard let points = Double(margin.stringValue), points.isFinite, (18...144).contains(points) else {
            throw NSError(domain: "Yu.Export.PDF", code: 1, userInfo: [NSLocalizedDescriptionKey: "PDF 页边距必须为 18–144 pt。"])
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
