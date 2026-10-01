# Windows 第四组原生预检记录：2026-10-01

**结论：当前原生门禁失败，尚不能开始完整 IME 人工验收或宣布第四组结项。**
两个运行时问题均已单独复现，根因尚未定位。此记录不修改产品实现。

以上是初次预检时的状态。同日后续已修复并通过原生门禁，详见
[原生阻塞修复记录](windows-group4-native-fix-20261001.md)；本页保留最初失败证据。

## 基线与环境

- 日期：2026-10-01，Asia/Shanghai。
- 代码：`83e1bdb27740f8cc623622117316fd596332702a`，当前 `main` 与本地 `origin/main` 引用一致。
  未执行远端 fetch，本记录不声称实时读取远端。
- 测试前工作区干净；本轮新增验收计划/样本并更新文档与 CRLF 样本属性，没有修改 Rust 产品代码。
- Windows 11 专业版 10.0.26100，x64；本轮操作者确认使用本机键鼠，单显示器。
- Intel Iris Xe 驱动 `32.0.101.7082`；设备列表另有 Remote Display Adapter / Oray，
  仅凭设备列表不能判定实际渲染适配器。
- Rust `1.98.1-x86_64-pc-windows-msvc`，rustc `48a229cea`。
  初次安装出现组件冲突/不完整，重新安装本轮新建的该工具链后，native 编译和测试可运行。
- 构建 exe：`target/debug/yu-shell-windows.exe`，PE32+、machine `8664`、
  subsystem `2`（Windows GUI）。
- exe SHA256：`5273270586A0A6EABFA2527506C0655CB7DA1556F815288D83456BE81C1CDA71`。

## 实际结果

| 项目 | 结果 | 说明 |
| --- | --- | --- |
| yu-font-windows native 测试 | FAIL | 17 passed / 1 failed |
| yu-render-windows 测试 | PASS | 2/2；验证 command/alignment，不等于 GPU 运行通过 |
| yu-shell-windows 测试 | PASS | 14/14 |
| cargo check -p yu-shell-windows | PASS | 原生 Windows 编译 |
| shell exe 构建 / PE 结构 | PASS | 原生链接成功，GUI x86-64 |
| --window-self-check | FAIL | cargo run 与直接执行 exe 都退出 1 |
| cargo test -p yu-editor | PASS | 汇总 31 个 test result：573 passed、0 failed、1 ignored，包含 doc-test |
| 真实 IME composition/candidate/commit/cancel | NOT_RUN | smoke 失败，尚未进行人工操作 |
| 单屏 100% / 125% / 150% DPI | NOT_RUN | 计划已准备 |
| 跨显示器不同 DPI | BLOCKED | 当前只有一块显示器 |

## 问题 W-PRE-01：真实 DirectWrite RTL shaping 失败

完整字体测试和单独运行下面测试均失败：

```powershell
cargo test -p yu-font-windows directwrite::tests::direct_write_shapes_rtl_and_complex_script_runs -- --exact --nocapture
```

实际错误：

```text
"שלום": native shaping backend failed: cluster map is not monotonic for the run direction at text unit 1
```

失败点：`platform/windows/yu-font-windows/src/directwrite.rs:931`。
单独运行退出码 101；这排除了“仅在该批测试并发执行时才出现”的解释。
本轮没有确定是 native cluster 翻译、方向处理还是其他条件导致。
测试在首个 Hebrew 样本失败，不能据此声称后续 Arabic / Devanagari 样本已通过。

## 问题 W-PRE-02：真实窗口 self-check 退出 1

两条入口均失败：

```powershell
cargo run -p yu-shell-windows -- --window-self-check
& ./target/debug/yu-shell-windows.exe --window-self-check
```

实际 stderr：

```text
Yu: 操作成功完成。 (0x00000000)
```

进程退出码为 **1**；错误消息中的“操作成功完成”不代表 smoke 成功。
现有日志没有失败阶段上下文，尚不能定位至 HWND、DirectWrite、D3D、TSF 初始化中的哪一步，
也不能确认本次已经成功 Present。
应先补充最小诊断或调试定位真实失败阶段，再修复并重跑自检。

## 证据与后续

本机证据位于被 Git 忽略的 `artifacts/windows-group4/20261001-preflight/`：

- `environment.json`：环境与人工测试状态。
- `native-self-check-retry.log` / `native-exit-code.txt`：完整门禁在字体测试处失败，退出 101。
- `rtl-repro.log`：单独 RTL 测试复现。
- `window-smoke.log`：直接执行窗口自检的 stderr，退出 1 已由执行器确认。
- `editor-regression.log`：完整 yu-editor 回归输出。
- `remaining-native.log`：部分 Transcript；自动执行环境没有完整捕获 native 输出，
  不单独依赖此文件证明各项通过。结果表同时依据执行器的输出与退出码。

建议顺序：先定位/修复 W-PRE-02，让窗口可运行；再修复 W-PRE-01 并补绿原生门禁；
随后按 [人工验收计划](windows-group4-manual-acceptance.md) 执行。
可独立推进语言组件和测试环境准备，但不能把尚未执行的 IME/DPI 项记为通过。
