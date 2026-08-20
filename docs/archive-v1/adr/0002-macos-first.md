# ADR 0002：macOS 是第一产品平台

- 状态：Accepted
- 日期：2026-08-09

## 决策

macOS 是 Yu 第一个达到产品质量的平台。共享核心仍保持平台无关，Windows 与 Linux 从第一阶段
开始保留编译和接口约束，但不要求同步完成全部原生行为。

macOS 风险实验直接使用 AppKit 和 `NSTextInputClient`，并允许最终由 Swift/Objective-C 或
Rust 原生绑定实现。

## 结果

- 首个完整垂直切片优先验证 AppKit 生命周期、IME、字体和 Accessibility；
- 平台类型不得进入 `yu-text`、`yu-markdown` 等核心 crate；
- 实验代码不视为产品平台层，可以在协议稳定后重写；
- Windows TSF 与 Linux IME 不因 macOS 优先而从架构中消失。

