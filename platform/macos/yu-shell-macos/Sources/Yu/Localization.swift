import Foundation
import Darwin

/// App-facing localization stays in the native shell. Rust owns document
/// semantics and machine-readable diagnostics, while user-visible wording is
/// resolved from the application bundle.
enum L10n {
    static func tr(_ key: String) -> String {
        Bundle.main.localizedString(forKey: key, value: key, table: "Localizable")
    }

    static func format(_ key: String, _ arguments: CVarArg...) -> String {
        String(format: tr(key), locale: Locale.current, arguments: arguments)
    }

    /// Used by localization self-checks to verify every shipped language
    /// without changing the user's preferred language.
    static func tr(_ key: String, language: String, bundle: Bundle = .main) -> String? {
        guard let path = bundle.path(forResource: language, ofType: "lproj"),
              let localizedBundle = Bundle(path: path) else { return nil }
        return localizedBundle.localizedString(forKey: key, value: nil, table: "Localizable")
    }
}

func runLocalizationSelfCheck() {
    let probes: [(String, String, String)] = [
        ("en", "Settings…", "Settings…"),
        ("zh-Hans", "Settings…", "设置…"),
        ("zh-Hant", "Settings…", "設定…"),
        ("ja", "Settings…", "設定…"),
        ("ko", "Settings…", "설정…"),
        ("en", "Help", "Help"),
        ("zh-Hans", "Help", "帮助"),
        ("zh-Hant", "Help", "說明"),
        ("ja", "Help", "ヘルプ"),
        ("ko", "Help", "도움말"),
        ("en", "Report an Issue…", "Report an Issue…"),
        ("zh-Hans", "Report an Issue…", "反馈问题…"),
        ("zh-Hant", "Report an Issue…", "回報問題…"),
        ("ja", "Report an Issue…", "問題を報告…"),
        ("ko", "Report an Issue…", "문제 신고…"),
        ("en", "Export failed. No output was committed.", "Export failed. No output was committed."),
        ("zh-Hans", "Export failed. No output was committed.", "导出失败，没有提交新的输出文件。"),
        ("zh-Hant", "Export failed. No output was committed.", "匯出失敗，沒有提交新的輸出檔案。"),
        ("ja", "Export failed. No output was committed.", "書き出しに失敗しました。新しい出力は保存されていません。"),
        ("ko", "Export failed. No output was committed.", "내보내기에 실패했습니다. 새 출력은 저장되지 않았습니다."),
    ]
    for (language, key, expected) in probes {
        guard let actual = L10n.tr(key, language: language), actual == expected else {
            fputs("localization-self-check failed: \(language) \(key)\n", stderr)
            exit(1)
        }
    }
    let available = Set(Bundle.main.localizations)
    guard Set(["en", "zh-Hans", "zh-Hant", "ja", "ko"]).isSubset(of: available) else {
        fputs("localization-self-check failed: bundle localizations = \(available.sorted())\n", stderr)
        exit(1)
    }
    print("localization-self-check: OK")
}
