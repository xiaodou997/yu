import AppKit

@main
struct NativeSpellingChecks {
    static func main() {
        _ = NSApplication.shared
        let checker = NSSpellChecker.shared
        let previousLanguage = checker.language()
        precondition(checker.setLanguage("en"))
        defer { _ = checker.setLanguage(previousLanguage) }
        let service = NativeSpellingService()
        let source = "中文 🙂 speling `codde`"
        let text = source as NSString
        let word = text.range(of: "speling")
        let suggestions = service.suggestions(source: source, allowed: [word], at: word.location + 2,
            revision: 42, language: "en_US")
        let defaultSuggestions = service.suggestions(source: source, allowed: [word], at: word.location + 2, revision: 42)
        precondition(defaultSuggestions.contains { $0.replacement == "spelling" })
        precondition(suggestions.contains { $0.replacement == "spelling" })
        precondition(service.suggestions(source: source,
            allowed: [NSRange(location: word.location + 1, length: 3)], at: word.location + 2,
            revision: 42, language: "en_US").isEmpty)
        precondition(suggestions.allSatisfy { $0.range == word && $0.revision == 42 })
        precondition(service.suggestions(source: source, allowed: [word], at: text.range(of: "codde").location,
            revision: 42, language: "en_US").isEmpty)
        precondition(service.suggestions(source: "spelling", allowed: [NSRange(location: 0, length: 8)], at: 2,
            revision: 42, language: "en_US").isEmpty)
        var cancelledPublished = false
        let cancelledRequest = service.requestDiagnostics(source: source, allowed: [word], isCancelled: { true }) { _ in cancelledPublished = true }
        precondition(cancelledRequest == -1 && !cancelledPublished)
        var asynchronous: [NSRange]?
        let sentence = "This sentence contains a speling mistake. `codde`"
        let prose = (sentence as NSString).range(of: "This sentence contains a speling mistake.")
        let sentenceWord = (sentence as NSString).range(of: "speling")
        service.requestDiagnostics(source: sentence, allowed: [prose]) { asynchronous = $0 }
        let deadline = Date().addingTimeInterval(10)
        while asynchronous == nil && Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.02)) }
        precondition(asynchronous == [sentenceWord], "Asynchronous diagnostics must exclude syntax and preserve UTF-16 source offsets: \(String(describing: asynchronous)), expected \(sentenceWord)")
        asynchronous = nil
        service.requestDiagnostics(source: source, allowed: [word]) { asynchronous = $0 }
        let isolatedDeadline = Date().addingTimeInterval(10)
        while asynchronous == nil && Date() < isolatedDeadline { RunLoop.current.run(until: Date().addingTimeInterval(0.02)) }
        precondition(asynchronous == [word], "Isolated words must use the current system language without changing its preferences")
        print("Native spelling: system suggestions, Unicode source offsets and excluded ranges passed")
    }
}
