import Foundation

// 面板上的一行文字：拿 canonical 镜像减掉被藏起来的区间。
//
// # 为什么这一步在平台侧，而且只能有一份
//
// 「哪几段被藏了」的唯一实现在 Rust 的 `DecorationSet` 里（不变量 D1），
// FFI 只回报区间（`yu_storage_session_block_hidden_spans`），不交出文本——
// 交出文本会破 C4「parser 不复制正文」与整套 range-backed 设计，而平台手上
// 本来就有 canonical 镜像。
//
// 于是剩下的「减一下」这一步落在这里。它有两个消费者（大纲面板、搜索结果
// 面板），各写一份必定分叉，而分叉的表现是两个面板上同一条标题显示得不一样
// ——不报错。所以这里是它的唯一实现。
//
// # 判据的分工
//
// 「藏对了没有」**不在这里证**：那是 `yu-decoration/src/hidden.rs` 的线性
// 参照实现与 `extension_decorations.rs` 那 45 条压住的事。这一层可能错的是
// 别的：UTF-16 偏移、区间重叠、逆序、越界。所以它的判据是**性质**，见
// `SelfChecks.swift` 的 `runPanelLabelSelfCheck`。
enum PanelLabel {
    /// `range` 那一段源码，减掉 `hidden` 覆盖的部分。
    ///
    /// `hidden` 必须升序、不重叠、并且整个落在 `range` 里——这正是
    /// `DecorationSet::hidden_spans()` 保证的形状。**不满足就整段原样返回**，
    /// 不去猜调用方的意思：显示源码是这一刀之前的行为，是一件真事；而按一组
    /// 自相矛盾的区间去减会得到一个谁也说不清的字符串，那才是静默地做错事。
    static func stripping(
        _ hidden: [NSRange],
        from source: NSString,
        in range: NSRange
    ) -> String {
        guard range.location >= 0,
              range.length >= 0,
              range.location + range.length <= source.length else {
            return ""
        }
        let raw = source.substring(with: range)
        guard !hidden.isEmpty else { return raw }
        guard isWellFormed(hidden, within: range) else { return raw }

        var result = ""
        var cursor = range.location
        for span in hidden {
            if span.location > cursor {
                result += source.substring(
                    with: NSRange(location: cursor, length: span.location - cursor)
                )
            }
            cursor = span.location + span.length
        }
        let tail = range.location + range.length
        if tail > cursor {
            result += source.substring(with: NSRange(location: cursor, length: tail - cursor))
        }
        return result
    }

    /// 升序、不重叠、非负长度、整个落在 `range` 里。
    ///
    /// 空区间（长度 0）是允许的：一条覆盖范围被删空的隐藏装饰不藏任何东西，
    /// 减掉它等于什么也不减。
    static func isWellFormed(_ hidden: [NSRange], within range: NSRange) -> Bool {
        var cursor = range.location
        for span in hidden {
            guard span.length >= 0, span.location >= cursor else { return false }
            guard span.location + span.length <= range.location + range.length else {
                return false
            }
            cursor = span.location + span.length
        }
        return true
    }
}
