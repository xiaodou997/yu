import AppKit

/// System word segmentation and dictionaries; Markdown eligibility comes from Rust.
final class NativeSpellingService {
    private let tag = NSSpellChecker.uniqueSpellDocumentTag()
    deinit { NSSpellChecker.shared.closeSpellDocument(withTag: tag) }

    struct Suggestion {
        let revision: UInt64
        let range: NSRange
        let replacement: String
    }

    /// Bounded chunks only; the caller binds completion to document revision
    /// and request generation. Syntax is blanked before the system sees it.
    @discardableResult
    func requestDiagnostics(source: String, allowed: [NSRange], isCancelled: @escaping () -> Bool = { false }, completion: @escaping ([NSRange]) -> Void) -> Int {
        guard !isCancelled() else { return -1 }
        let original = source as NSString
        let safe = allowed.filter { $0.location >= 0 && $0.length > 0 && $0.location <= original.length && $0.length <= original.length - $0.location }
        let masked = NSMutableString(string: String(repeating: " ", count: original.length))
        for range in safe { masked.replaceCharacters(in: range, with: original.substring(with: range)) }
        let checker = NSSpellChecker.shared
        let orthography = NSOrthography.defaultOrthography(forLanguage: checker.language())
        return checker.requestChecking(of: masked as String, range: NSRange(location: 0, length: masked.length),
            types: NSTextCheckingResult.CheckingType.spelling.rawValue, options: [.orthography: orthography],
            inSpellDocumentWithTag: tag) { [weak self] _, results, detected, _ in
                let ranges = results.filter { $0.resultType == .spelling }.map(\.range).filter { hit in
                    hit.location != NSNotFound && hit.length > 0 && safe.contains { span in
                        hit.location >= span.location && NSMaxRange(hit) <= NSMaxRange(span)
                    }
                }
                DispatchQueue.main.async { [weak self] in
                    guard let self, !isCancelled() else { return }
                    if detected.dominantLanguage == "und" {
                        // Isolated words provide insufficient language context.
                        // Check one token per run-loop turn in the user's current
                        // language, without changing global spell preferences.
                        self.checkUndeterminedWords(source: source, allowed: safe, isCancelled: isCancelled, completion: completion)
                    } else { completion(ranges) }
                }
            }
    }

    private func checkUndeterminedWords(source: String, allowed: [NSRange], isCancelled: @escaping () -> Bool, completion: @escaping ([NSRange]) -> Void) {
        let text = source as NSString
        let language = NSSpellChecker.shared.language()
        var words: [NSRange] = []
        text.enumerateSubstrings(in: NSRange(location: 0, length: text.length), options: [.byWords, .substringNotRequired]) { _, range, _, _ in
            if allowed.contains(where: { range.location >= $0.location && NSMaxRange(range) <= NSMaxRange($0) }) { words.append(range) }
        }
        var cursor = 0
        var diagnostics: [NSRange] = []
        func next() {
            guard !isCancelled() else { return }
            guard cursor < words.count else { completion(diagnostics); return }
            let word = words[cursor]
            cursor += 1
            let result = NSSpellChecker.shared.checkSpelling(of: text.substring(with: word), startingAt: 0,
                language: language, wrap: false, inSpellDocumentWithTag: tag, wordCount: nil)
            if result.location != NSNotFound, result.length > 0 {
                diagnostics.append(NSRange(location: word.location + result.location, length: result.length))
            }
            DispatchQueue.main.async { next() }
        }
        next()
    }

    func suggestions(source: String, allowed: [NSRange], at offset: Int, revision: UInt64,
                     language: String? = nil) -> [Suggestion] {
        let text = source as NSString
        guard let span = allowed.first(where: { NSLocationInRange(offset, $0) }),
              NSMaxRange(span) <= text.length else { return [] }
        var word: NSRange?
        let paragraph = text.paragraphRange(for: NSRange(location: offset, length: 0))
        text.enumerateSubstrings(in: paragraph, options: [.byWords, .substringNotRequired]) { _, range, _, stop in
            if NSLocationInRange(offset, range) { word = range; stop.pointee = true }
            else if range.location > offset { stop.pointee = true }
        }
        guard let word, word.location >= span.location, NSMaxRange(word) <= NSMaxRange(span) else { return [] }
        let value = text.substring(with: word)
        let checker = NSSpellChecker.shared
        let language = language ?? checker.language()
        let misspelled = checker.checkSpelling(of: value, startingAt: 0, language: language,
            wrap: false, inSpellDocumentWithTag: tag, wordCount: nil)
        guard misspelled.location != NSNotFound, misspelled.length > 0 else { return [] }
        let diagnostic = NSRange(location: word.location + misspelled.location, length: misspelled.length)
        guard NSLocationInRange(offset, diagnostic) else { return [] }
        return (checker.guesses(forWordRange: misspelled, in: value, language: language,
            inSpellDocumentWithTag: tag) ?? []).prefix(8).map {
                Suggestion(revision: revision, range: diagnostic, replacement: $0)
            }
    }
}
