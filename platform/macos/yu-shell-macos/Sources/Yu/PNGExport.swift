import AppKit

/// Independent image preferences. Shared style/base controls still resolve the
/// existing document theme; image dimensions never depend on the live window.
@MainActor
final class NativePNGExportOptions {
    let view = NSStackView()
    private let common: NativeHTMLExportOptions
    private let width: NSTextField
    private let scale = NSPopUpButton()
    init(untitled: Bool) {
        common = NativeHTMLExportOptions(untitled: untitled, preferenceKey: "Yu.exportPNG.currentTheme")
        let preferences = UserDefaults.standard
        let reading = NativeWritingPreferences.shared
        let fallback = reading.columnWidth > 0 ? reading.columnWidth : 800
        width = NSTextField(string: String(preferences.object(forKey: "Yu.exportPNG.width") == nil ? Int(min(2048, max(320, fallback))) : preferences.integer(forKey: "Yu.exportPNG.width")))
        width.setAccessibilityIdentifier("yu-png-width")
        width.widthAnchor.constraint(equalToConstant: 72).isActive = true
        scale.addItems(withTitles: ["1×", "2×"])
        scale.selectItem(at: preferences.integer(forKey: "Yu.exportPNG.scale") == 2 ? 1 : 0)
        scale.setAccessibilityIdentifier("yu-png-scale")
        view.orientation = .vertical; view.alignment = .leading; view.spacing = 9
        view.addArrangedSubview(common.view)
        view.addArrangedSubview(NSStackView(views: [NSTextField(labelWithString: L10n.tr("Width (320–2048):")), width, scale]))
        let note = NSTextField(wrappingLabelWithString: L10n.tr("A single image is limited to 16 Mi pixels and a 32768-pixel edge. Oversized documents can be confirmed for segmented export to a new “name-images” directory. The complete export is limited to 128 Mi pixels / 64 segments and never overwrites an existing directory."))
        note.preferredMaxLayoutWidth = 340
        note.widthAnchor.constraint(equalToConstant: 340).isActive = true
        view.addArrangedSubview(note)
        view.widthAnchor.constraint(equalToConstant: 360).isActive = true
        view.frame = NSRect(x: 0, y: 0, width: 360, height: untitled ? 245 : 170)
    }
    func config(title: String, untitled: Bool, appearance: NSAppearance) throws -> [String: Any] {
        guard let logical = Int(width.stringValue), (320...2048).contains(logical) else {
            throw NSError(domain: "Yu.Export.PNG", code: 1, userInfo: [NSLocalizedDescriptionKey: L10n.tr("PNG width must be between 320 and 2048.")])
        }
        let multiplier = scale.indexOfSelectedItem == 1 ? 2 : 1
        var config = common.config(title: title, untitled: untitled, appearance: appearance)
        config["exportFormat"] = "png"; config["width"] = logical; config["scale"] = multiplier
        UserDefaults.standard.set(logical, forKey: "Yu.exportPNG.width")
        UserDefaults.standard.set(multiplier, forKey: "Yu.exportPNG.scale")
        return config
    }
}
