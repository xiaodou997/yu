import Foundation

/// Shared preference keys, independent from any document's source or history.
struct NativeWritingPreferences {
    enum ImagePolicy: String { case copy, reference }
    static let shared = NativeWritingPreferences(defaults: .standard)
    static let didChange = Notification.Name("Yu.writingPreferencesChanged")
    let defaults: UserDefaults

    var spellingEnabled: Bool {
        get { defaults.object(forKey: "Yu.spellingEnabled") as? Bool ?? true }
        nonmutating set { defaults.set(newValue, forKey: "Yu.spellingEnabled") }
    }

    var focusMode: Bool {
        get { defaults.bool(forKey: "Yu.focusMode") }
        nonmutating set { defaults.set(newValue, forKey: "Yu.focusMode") }
    }

    var typewriterMode: Bool {
        get { defaults.bool(forKey: "Yu.typewriterMode") }
        nonmutating set { defaults.set(newValue, forKey: "Yu.typewriterMode") }
    }

    var fontSize: Double {
        get {
            let value = defaults.object(forKey: "Yu.bodyFontSize") as? Double ?? 16
            return value.isFinite && (12...32).contains(value) ? value : 16
        }
        nonmutating set {
            guard newValue.isFinite, (12...32).contains(newValue) else { return }
            defaults.set(newValue, forKey: "Yu.bodyFontSize")
        }
    }

    /// Zero preserves each theme's default column and wide-window breakpoints.
    var columnWidth: Double {
        get {
            let value = defaults.object(forKey: "Yu.readingColumnWidth") as? Double ?? 0
            return value.isFinite && (360...1200).contains(value) ? value : 0
        }
        nonmutating set {
            guard newValue == 0 || (newValue.isFinite && (360...1200).contains(newValue)) else { return }
            defaults.set(newValue, forKey: "Yu.readingColumnWidth")
        }
    }

    func resetReadingPreferences() {
        for key in ["Yu.bodyFontSize", "Yu.readingColumnWidth", "Yu.typewriterMode", "Yu.focusMode", "Yu.spellingEnabled"] { defaults.removeObject(forKey: key) }
    }

    var imagePolicy: ImagePolicy {
        get { ImagePolicy(rawValue: defaults.string(forKey: "Yu.imagePolicy") ?? "") ?? .copy }
        nonmutating set { defaults.set(newValue.rawValue, forKey: "Yu.imagePolicy") }
    }

    var imageDirectory: String {
        let value = defaults.string(forKey: "Yu.imageDirectory") ?? "assets"
        return NativeImageResources.isValidDirectory(value) ? value : "assets"
    }

    func setImageDirectory(_ value: String) throws {
        guard NativeImageResources.isValidDirectory(value) else { throw NativeImageResources.Failure.invalidDirectory }
        defaults.set(value, forKey: "Yu.imageDirectory")
    }

    func resetImagePreferences() {
        for key in ["Yu.imagePolicy", "Yu.imageDirectory"] { defaults.removeObject(forKey: key) }
    }
}
