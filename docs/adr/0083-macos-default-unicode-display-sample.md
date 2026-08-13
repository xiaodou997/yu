# ADR-0083：macOS spike 默认显示 Unicode 验收样本

## 状态

已接受（2026-08-13）

## 背景

只在启动后要求用户切换日文输入源、按 dead key 或输入 emoji，无法快速判断字体 fallback、shaping、
换行和 Accessibility 文本暴露是否正常；而启动自检原本使用简单的 LTR 文本，直接替换成 RTL/emoji
样本会让旧的 caret round-trip 诊断把 BiDi 行误判为协议失败。

## 决策

窗口默认显示固定 Unicode 样本，覆盖中文、日文平假名/片假名/汉字、组合重音、dead-key 典型输出、
emoji、符号、阿拉伯文和希伯来文。启动自检检查必需片段存在，并验证 AX 字符数等于 Swift 字符串的
UTF-16 长度；实际窗口的 AX value 必须能读回完整样本。

启动期间的旧 layout/caret/viewport probe 使用独立的简单 LTR source。probe 完成后恢复 Unicode 样本，
再开始交互 IME 捕获。样本中的 `´ + e = é` 等只是静态预期显示，不模拟 dead key；真实日文输入法、
dead key 事件和 VoiceOver 朗读仍由人工验收完成。

## 影响

打开 spike 即可发现常见 Unicode 显示或 AX 回归，同时不牺牲已有协议 probe 的确定性。默认文档不写入
IME 事件日志中的 canonical source，也不会把静态显示检查误标为输入法通过。
