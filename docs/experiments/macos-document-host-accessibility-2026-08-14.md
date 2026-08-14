# macOS 文档 host Accessibility / 输入源人工验收：2026-08-14

这份记录对应 `experiments/macos-document-host`，用于验证可写 native mirror 接入统一
`DocumentEditorSession` 后的真实 macOS 输入源和 VoiceOver 行为。它不自动切换输入源、不修改
VoiceOver 设置，也不把静态 Unicode 文本当作输入法通过。

## 启动

先构建 FFI 和 app：

```bash
experiments/macos-document-host/build-app.sh
open experiments/macos-document-host/.build/YuMacDocumentHost.app --args \
  "$PWD/docs/experiments/macos-document-host-accessibility-2026-08-14.md"
```

也可以直接运行 Swift package：

```bash
experiments/macos-document-host/build-rust-ffi.sh
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  "$PWD/docs/experiments/macos-document-host-accessibility-2026-08-14.md"
```

在开启 VoiceOver 前，先跑无窗口 semantic child 契约自检：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --accessibility-self-check \
  "$PWD/experiments/macos-document-host/Fixtures/sample.md"
```

## 场景

| 场景 | 手动选择 | 操作 | 通过条件 |
| --- | --- | --- | --- |
| 日文 Romaji | `Japanese - Romaji` / Kotoeri | 在文档末尾输入 `nihongo`，观察平假名 preedit，按 Space/Return 提交 `日本語` | marked text 可见且不重复；commit 只产生一次 canonical insertion |
| 日文 Kana | `Japanese - Kana`（若启用） | 输入一个假名词并提交，再输入一次后按 Escape | cancel 后 source、AX value、selection 回到提交前 |
| dead key | 当前系统可用的 dead-key 键盘源 | 生成 `é`、`à` 或 `ñ`，再用 Backspace 删除 | 组合字符作为稳定文本；退格不拆 grapheme |
| combining mark | `ABC` 或可直接输入 Unicode 的源 | 输入 `e` + `U+0301`，或粘贴 `e\u{301}` | AX UTF-16 range 不落在 surrogate/scalar 中，删除行为可预测 |
| VoiceOver | 任意稳定键盘源 | 开启 VoiceOver，聚焦文本区，使用 Heading/Link Rotor、标题/列表导航，朗读中文/日文/emoji 和一段 preedit，并尝试按 task checkbox | 文本区及 semantic children 角色、label、父子导航正确；Rotor 能定位标题/链接；task 状态可读且 press 只切换 marker；URL 可被发现但不会自动打开；canonical value 和选区可朗读；关闭 VoiceOver 后不改变 source |

输入源名称和 identifier 会因 macOS 版本、地区和用户配置变化，记录实际值，不写死示例名称。

## 自动证据

Rust FFI 回归覆盖：

- Accessibility snapshot 的 Revision、UTF-16 字符数、selection、line count；
- line range 和 position 查询；
- source 修改后旧 Revision 返回 `YU_STORAGE_STALE_REVISION`。

Swift host 的 `--accessibility-self-check` 另外覆盖：

- 真实 AppKit Accessibility child 的 role、parent/children 和 source-backed label；
- Heading/Link custom rotor 返回当前 Revision 的标题/链接目标；task checkbox value 同时覆盖 todo/done；
- link destination 的 `accessibilityURL` 与 task checkbox press 的 Revision/旧 child 失效契约；
- 编辑后旧 child 的 Revision-bound label 失效，新树使用新 Revision；
- 中文、日文、emoji、combining mark 和 task-list fixture 的无窗口构造。

这些测试不代替 VoiceOver 朗读质量和候选窗口视觉观察。TextKit AX 查询使用 Rust snapshot，文本内容
通过 expected-Revision source range 读取；如果实现返回 stale，应该刷新查询，而不是读取本地 mirror
猜测结果。

## 记录

```text
Date/time:
macOS version:
Yu commit:
Keyboard / machine:
Scenario:
Input source name:
Input source identifier:

Observed:
- [ ] Text area role and label are visible to Accessibility Inspector.
- [ ] AX character count equals Rust UTF-16 snapshot.
- [ ] AX selection follows Rust selection after mouse/keyboard movement.
- [ ] Japanese preedit and candidate confirmation commit once.
- [ ] Dead key / combining mark keeps grapheme behavior.
- [ ] VoiceOver reads Chinese, Japanese, emoji and canonical source.
- [ ] No stale Revision is silently presented after an edit.

Unexpected output / recording:
Conclusion: PASS / FAIL / NEEDS FOLLOW-UP
```
