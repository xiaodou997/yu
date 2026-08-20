# macOS Unicode Composition Replay：2026-08-09

## 目标

在不依赖当前用户输入源的前提下，验证 `NSTextInputClient` 事件序列和 Rust
`CompositionOverlay` 对日文、dead-key/组合重音、commit 与 cancel 的处理边界。

## 回放序列

Swift spike 启动时保存完整 attributed storage，然后执行：

```text
setMarkedText("にほんご")
setMarkedText("にほんご")
insertText("日本語")

setMarkedText("\\u{301}")
setMarkedText("e◌́")
insertText("é")

setMarkedText("にほん")
cancel:
```

每次 `setMarkedText` 都只更新 marked range；commit 后 marked range 消失并推进 selection；
最后一次 cancel 恢复到 commit 后的正文。回放结束恢复初始 storage、selection 和 affinity。

## 实测输出

```text
setMarkedText preedit="にほんご" selection={4, 0} replace={47, 0}
insertText commit="日本語" replace={47, 4}
setMarkedText preedit="é" selection={2, 0} replace={50, 1}
insertText commit="é" replace={50, 2}
cancelComposition range={51, 3}
Unicode composition self-check japanese=日本語 combining=é cancel=restored
```

Rust `yu-editor` 的对应测试通过，并确认最终内容为 `输入: 日本語é`、revision 只增加两次。

## 结论与限制

- 日文 preedit、组合标记和取消都可以在 composition overlay 中表达；
- 组合重音的 UTF-16 selection 不拆 surrogate/scalar，commit 是唯一永久修改；
- 这是协议回放，不等同于切换 macOS 日文输入源后的真实候选窗体验；
- 当前用户会话的 VoiceOver 状态为 `off`，因此本次只验证 AX tree，不宣称 VoiceOver 朗读质量
  已通过。没有修改系统 VoiceOver 设置。
