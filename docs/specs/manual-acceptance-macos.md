# macOS 人工验收清单

自动化测试到不了的地方。这里列的每一条都需要人坐在机器前操作真实输入法、
真实 VoiceOver、真实鼠标——不是因为懒得写测试，而是因为被测的对象是
macOS 自己的行为。

验收对象是产品 app，不是任何实验壳：

```bash
platform/macos/yu-shell-macos/run-app.sh <文件>
```

> `open` 对运行中的 app 只激活不重启，改动后必须用 `run-app.sh`。

自动化覆盖的部分见 `tools/verify.sh`（Rust 测试 + 10 个 headless self-check）
与 `--launch-window-self-check`（真实 NSWindow 下的帧调度与可滚动范围）。

---

## A. IME

切换到真实输入源后逐条确认。Yu 不会替你切换输入法。

**A1. 拼音/罗马字的 preedit 只改变 marked range。**
输入过程中 canonical source 不变、Undo 里不出现中间态（不变量 H1、H2）。

**A2. 选词只产生一次 commit。**
不是「先删再插」，也不是每次候选变化都提交一次。

**A3. candidate window 跟随 caret。**
多行 preedit 时锚定在**第一个视觉 fragment**，不能因为取整个 marked range
的并集而跳到后面的行。

**A4. Escape 取消后正文完全恢复。**
包括 caret 位置与选区。

**A5. emoji 与组合字符退格不产生无效序列。**
`👨‍👩‍👧‍👦`、`é`（e + U+0301）、`🏳️‍🌈` 各删一次，看剩下的是不是合法字形
（不变量 H7）。

**A6. dead key。**
`´ + e = é`、`` ` + a = à``、`^ + o = ô`、`~ + n = ñ`。

建议的场景标签（便于记录）：`japanese-romaji`、`japanese-kana`、
`pinyin`、`combining-accents`、`dead-key`。

---

## B. Accessibility

需要开启 VoiceOver，并给终端或 app 授予 Accessibility 权限。

**B1.** 整份文档作为一个 `AXTextArea` 暴露，朗读顺序与视觉顺序一致。
**B2.** 移动光标时 VoiceOver 报出正确的行与位置。
**B3.** composition 进行中，AX 值、`NSTextInputClient` 与屏幕上画的是
同一份 overlay（不变量 H3）。
**B4.** 表格分隔线可用 VoiceOver 聚焦，increment/decrement 改变列宽且
不修改 Markdown 源码。

---

## C. 渲染

headless self-check 覆盖不到「画出来对不对」。

**C1.** 中文、日文、emoji、阿拉伯文、希伯来文都能显示，不出现豆腐块。
**C2.** Retina 与非 Retina 屏之间移动窗口，字形保持清晰。
**C3.** 缩放窗口后按新宽度重排，没有残留的旧帧。
**C4.** 长文档滚到底能看到最后一行；打开时停在开头。
**C5.** 图片解码完成后替换 placeholder，位置不跳。

---

## 记录

这些检查没有退出码。做完在 PR 或 commit 里写清楚**跑了哪几条、在什么
硬件和系统版本上**——「人工验收通过」不带范围等于没做。

v1 时期有一套把 IME 事件记成 JSON 再离线审计的工具（`IME_EVENT` 日志 +
`--audit-ime-log`），随 `experiments/macos-text-input` 一起删除了。它审计的
是那个实验壳自己的事件协议，而产品壳走的是另一条路径（`NSTextView` 子类）。
如果将来 IME 回归频繁到需要日志审计，应该基于产品壳的事件重建，而不是复活
那份实现。历史记录见 `docs/archive-v1/adr/0081-macos-ime-log-audit.md`。
