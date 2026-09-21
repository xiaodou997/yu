import AppKit
import CoreText
import YuStorageFFI

/// The document theme is independent of AppKit chrome appearance.
enum NativeTheme {
    enum Selection: Int { case yu, github, night }
    static let didChange = Notification.Name("Yu.documentThemeChanged")
    static var selection: Selection {
        get { Selection(rawValue: UserDefaults.standard.integer(forKey: "Yu.readingTheme")) ?? .yu }
        set { UserDefaults.standard.set(newValue.rawValue, forKey: "Yu.readingTheme") }
    }
    static func resolved(dark: Bool) -> UInt8 {
        switch selection {
        case .yu: return dark ? UInt8(YU_STORAGE_THEME_YU_DARK) : UInt8(YU_STORAGE_THEME_YU_LIGHT)
        case .github: return UInt8(YU_STORAGE_APPEARANCE_LIGHT)
        case .night: return UInt8(YU_STORAGE_APPEARANCE_DARK)
        }
    }
    static func spec(resolved: UInt8) -> YuStorageThemeSpec {
        var result = YuStorageThemeSpec()
        precondition(yu_storage_theme_spec(resolved, &result) == YU_STORAGE_OK)
        return result
    }

    static func spec(dark: Bool = false) -> YuStorageThemeSpec {
        var result = YuStorageThemeSpec()
        precondition(yu_storage_theme_spec(resolved(dark: dark), &result) == YU_STORAGE_OK)
        return result
    }

    static func reading(width: CGFloat, windowWidth: CGFloat, scrollY: CGFloat = 0, dark: Bool = false) -> YuStorageReadingGeometry {
        var result = YuStorageReadingGeometry()
        precondition(yu_storage_reading_geometry(resolved(dark: dark), Float(max(width, 1)), Float(max(windowWidth, 1)), Float(scrollY), Float(NativeWritingPreferences.shared.columnWidth), &result) == YU_STORAGE_OK)
        return result
    }

    static func color(_ key: KeyPath<YuStorageThemeSpec, UInt32>) -> NSColor {
        NSColor(name: nil) { appearance in
            let value = spec(dark: appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua)[keyPath: key]
            return NSColor(srgbRed: CGFloat((value >> 24) & 255) / 255,
                           green: CGFloat((value >> 16) & 255) / 255,
                           blue: CGFloat((value >> 8) & 255) / 255,
                           alpha: CGFloat(value & 255) / 255)
        }
    }

    /// Resolve the shared ThemeFont identity to an AppKit face.
    static func font(identity: UInt32, size: CGFloat) -> NSFont {
        if identity == 7 { return NSFont.systemFont(ofSize: size) }
        if identity == 8 { return NSFont.userFixedPitchFont(ofSize: size) ?? NSFont.monospacedSystemFont(ofSize: size, weight: .regular) }
        let families: [UInt32: String] = [1: "Open Sans", 2: "Helvetica Neue",
            3: "Lucida Grande", 4: "Menlo", 5: "Monaco", 6: "Courier"]
        return families[identity].flatMap { NSFont(name: $0, size: size) }
            ?? NSFont.systemFont(ofSize: size)
    }

    static func registerFonts() {
        guard let root = Bundle.main.resourceURL?.appendingPathComponent("Fonts"),
              let urls = try? FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil) else { return }
        for url in urls where url.pathExtension == "ttf" {
            var error: Unmanaged<CFError>?
            if !CTFontManagerRegisterFontsForURL(url as CFURL, .process, &error) {
                NSLog("Font registration: %@", error?.takeRetainedValue().localizedDescription ?? url.lastPathComponent)
            }
        }
    }
}
