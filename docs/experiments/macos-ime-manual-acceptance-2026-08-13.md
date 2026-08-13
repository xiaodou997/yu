# macOS IME 人工验收记录模板：2026-08-13

这份记录把协议级 self-check 与真实 macOS 输入源/VoiceOver 验收分开。它不自动切换输入源、不修改
VoiceOver 设置，也不把静态 Unicode 样本当成输入法通过；每个场景都必须留下原始日志、当前输入源和
人工观察结果。

## 使用方式

捕获脚本会自动构建并运行签名的临时 `.app`；如果只想先确认构建，也可以单独执行：

```bash
experiments/macos-text-input/build-app.sh
```

在“系统设置 → 键盘 → 输入法”中手动选择本次场景需要的输入源，然后运行捕获脚本。脚本只设置场景
标签，不会替用户切换输入法：

```bash
experiments/macos-text-input/run-manual-acceptance.sh \
  japanese-romaji /tmp/yu-ime-japanese-romaji.log
```

完成输入后先让 marked text 收敛（提交或取消），再按 `Ctrl-C` 结束捕获。随后执行严格审计：

```bash
experiments/macos-text-input/audit-manual-acceptance.sh \
  japanese-romaji /tmp/yu-ime-japanese-romaji.log
```

审计通过只表示事件协议、session 边界、range/generation 和 canonical Revision 关系正确；它不替代
下面的视觉、输入法候选或 VoiceOver 人工观察。

## 场景清单

| 场景标签 | 手动选择的输入源 | 操作 | 需要观察 |
| --- | --- | --- | --- |
| `japanese-romaji` | `Japanese - Romaji`（Kotoeri） | 在默认样本文档末尾输入 `nihongo`，观察平假名 preedit，再按 Space/Return 选词提交 `日本語`；再输入一段片假名并提交 | preedit 下划线、候选窗首个视觉 fragment 跟随 caret、一次 commit、正文无重复字符 |
| `japanese-kana` | `Japanese - Kana`（若系统已启用） | 使用键盘上的假名键输入一个平假名词，再提交；用 Escape 取消第二次 preedit | Kana preedit 与 Romaji 一样走 overlay；取消后正文和 selection 恢复 |
| `dead-key` | 已启用 dead key 的键盘源（例如 `U.S. International`；不要假设所有 ABC 都支持） | 按该输入源显示的死键组合生成 `é`、`à` 或 `ñ`；若使用常见布局，可尝试 Option+E 后 E、Option+N 后 N | 组合字符作为一次稳定文本提交；没有伪造的 `setMarkedText`；Unicode scalar/grapheme 不被拆开 |
| `combining-mark` | `ABC` 或当前可直接输入 Unicode 的源 | 在默认样本后输入 `e` 与组合重音 `U+0301`，或粘贴 `e\u{301}`；再用 Backspace 删除 | AX/selection 使用 UTF-16；退格按 grapheme 删除；`é`（预组字符）与 `e + U+0301` 都可被保留 |
| `voiceover` | 任意稳定键盘源（建议 `ABC`） | 手动开启 VoiceOver，聚焦文本区域，朗读标题、默认 Unicode 样本和一段 preedit；再关闭 VoiceOver | 文本区域被识别为一个 text entry/AXTextArea；中文、日文、emoji、组合字符可被读到；marked text 不造成重复朗读 |

输入源名称可能因 macOS 版本、地区和用户配置不同。记录实际显示的名称/identifier，不要把示例名称
当作硬编码前置条件。

## 每次运行记录

复制下面的段落到 issue 或本文件的副本中，填写真实结果：

```text
Date/time:
macOS version:
Yu commit:
Machine/keyboard:
Scenario:
Selected input source name:
Selected input source identifier:
Log path:
Strict audit output:

Observed:
- [ ] Default Unicode sample visible on first open.
- [ ] Keyboard input source probe matches the selected source.
- [ ] Preedit/marked text is visible and underlined where expected.
- [ ] Candidate window follows the caret.
- [ ] Commit produces one canonical insertion.
- [ ] Escape/cancel restores the prior canonical text.
- [ ] Backspace does not split a grapheme unexpectedly.
- [ ] AX text area exposes the expected value and UTF-16 length.
- [ ] VoiceOver reads the intended text (voiceover scenario only).

Unexpected output / screen recording reference:
Conclusion: PASS / FAIL / NEEDS FOLLOW-UP
```

## 证据边界

- `Default display sample self-check` 和 AX runtime probe 是启动时自动证据，证明显示、shaping 和
  文本暴露的固定片段存在；它们不证明日文输入源、dead key 或 VoiceOver。
- `IME audit passed ... --expect-scenario NAME` 是日志协议证据，证明这份日志确实带有预期场景标签，且
  composition 生命周期没有破坏 canonical source；它不判断候选词是否“选对”。
- 日文候选选择、dead key 实际按键、VoiceOver 朗读质量都必须由人观察并填写本模板。
- 原始日志可以放在 `/tmp` 或本地未跟踪目录；仓库只提交模板、脚本和不含个人输入内容的最小 fixture。
