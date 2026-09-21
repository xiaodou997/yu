# Mac 第四组现状与验收入口

更新：2026-09-21。第四组尚未完成。本页是当前状态入口；逐轮实现细节保留在 [实现记录](mac-extended-document.md)，逐项证据见 [验收矩阵](mac-extended-document-acceptance.md)。

## 当前结论

原生公式、图表和扩展文档的生产链路已经接通，不是“还没有实现”。剩余工作集中在组合行为、明确支持边界，以及最新构建的统一实窗验收。此前记录按迭代轮次累积，导致整体状态难以判断；后续以本页的任务清单跟踪，历史截图不能直接证明新构建完成。

当前桌面预检为已解锁，辅助功能、事件发送和录屏权限均可用。早先的锁屏阻塞已解除；本次按用户要求优先整理、验证和提交代码，不将这次预检计作实窗验收。

## 已实现

| 功能 | 当前能力 |
| --- | --- |
| 原生公式 | MiTeX＋Typst 矢量输出；行内/块、分式、根式、矩阵、多行对齐、中文、编号引用；错误诊断与源码保留 |
| 原生图表 | Mermaid 七类基础图与扩展语料；ER 别名/属性；时序半箭头、Actor、创建/销毁；多项 Gantt 排程修复 |
| 辅助进程 | 随包、按需启动、版本校验、取消、空闲退出；旧简化公式模块已移除 |
| 扩展文档 | 脚注、目录、front matter、高亮、上下标；有限 HTML 段落、列表、表格、图片、对齐及折叠内容 |
| 结构编辑 | 合并表格布局/导航/结构修改/剪贴板往返；Markdown 表格接收合并内容；多种 HTML 列表选区编辑 |
| Mac 写作功能 | 图片插入/拖放/粘贴/尺寸、专注/打字机、拼写检查和设置；此前第三组已有本机验收记录 |

最新修复：`B-->>-A` 正确结束发送者 B 的激活状态。新增嵌套调用和实际激活条几何回归。中心连接 `()` 只有语义调研，没有实现，不能列入已完成项。

## 还没做完

1. **最新构建的综合实窗验收**：浅深色、真实中文输入、点击/拖选、撤销重做、保存与完整退出重开；合并表格拖选/列宽以及写作/图片回归。现有原生自检和独立 SVG 图像不替代这些操作。
2. **HTML 结构组合**：不同层级交叠列表选择；合并内容跨目标 HTML 行组粘贴；含公式/脚注的 Markdown 表格转换。当前拒绝的操作必须保持源码和历史，不得伪成功。
3. **图表边界**：中心连接 `()`、更多时序生命周期组合；Gantt 时分秒与部分格式/配置，跨日和唤醒刷新。根据原计划逐项给出支持范围与明确诊断，不以“能生成 SVG”代替语义正确性。
4. **第四组最终审计**：绑定同一应用/辅助程序哈希，汇总公式、图表、脚注、目录、HTML、错误恢复、取消与资源回收证据，完成后才能结项。

后续组别不计入第四组完成：第五组是 HTML/PDF/打印/图片/Pandoc 导出；第六组是 macOS 26 实机、VoiceOver、CI 环境、签名公证和分发更新。完整产品规划见 [路线图](mac-product-roadmap.md)。

## 测试文档是哪个

**人工集中查看使用 [group4-smoke.md](../../platform/macos/yu-shell-macos/Fixtures/group4-smoke.md)**。它包含公式、七类图表、脚注/目录、扩展文字、HTML 列表/合并表格/details、本地图片及两项故意出错的语料。末尾未知公式和未知图表应显示诊断，这是预期，不是正常内容渲染失败。

请先复制此文件再编辑，以保留仓库基准。相对图片路径使用同级 `assets/yu-mark.png`，复制到其他目录时应同时保留 assets 目录。新增固定语料尚未完成整窗验收，不能因它被提交就视为验收通过。

其他入口：

- [render-embedded.md](../../platform/macos/yu-shell-macos/Fixtures/render-embedded.md)：原有最小公式/流程图样本。
- [native-parity-reference.md](../../platform/macos/yu-shell-macos/Fixtures/native-parity-reference.md)：早期排版对照，已不以 Typora 像素级复刻为验收目标。
- [run-embedded-checks.py](../../platform/macos/yu-shell-macos/run-embedded-checks.py)：真正的外部事件自动化；各场景生成隔离临时文档，操作结果和截图写入指定输出目录。
- [SelfChecks.swift](../../platform/macos/yu-shell-macos/Sources/Yu/SelfChecks.swift)：原生宿主/FFI/剪贴板/保存行为检查。
- [helper 测试](../../tools/yu-document-renderer/src/lib.rs)、[HTML 编辑测试](../../crates/yu-editor/tests/html_table_edit.rs)：语义、几何和源码事务回归。

从仓库根目录运行（输出目录必须尚不存在）：

```sh
cargo test --workspace
platform/macos/yu-shell-macos/run-self-checks.sh
python3 platform/macos/yu-shell-macos/run-embedded-checks.py --math-suite --reopen artifacts/group4-math-current-light
python3 platform/macos/yu-shell-macos/run-embedded-checks.py --math-suite --dark --reopen artifacts/group4-math-current-dark
python3 platform/macos/yu-shell-macos/run-embedded-checks.py --diagram-suite --reopen artifacts/group4-diagrams-current-light
python3 platform/macos/yu-shell-macos/run-embedded-checks.py --diagram-suite --dark --reopen artifacts/group4-diagrams-current-dark
python3 platform/macos/yu-shell-macos/run-embedded-checks.py --html-blocks --merged-tables --reopen artifacts/group4-tables-current-light
```

以上只列主要入口，不等同完整最终矩阵。外部事件测试需要解锁并独占前台输入；截图必须实际复核。生成的测试应用、截图与临时文档位于 Git 忽略的 `artifacts/`，不提交到仓库。

## 本次提交前验证

- 完整 Rust workspace：1444 项通过、5 项忽略、0 失败，包含 75 项 helper 库测试。
- 全工作区 Clippy、Rust 格式检查通过。
- CI 命令映射、依赖方向、FFI 头文件、条件依赖、rope 边界及坐标检查通过。
- Release 构建及应用审计通过；当前应用 SHA256：`1731a774376a3774ed957a4dd2074754ff061504464ae00153f0ad51e632d49f`。
- 最新应用19项原生自检通过；Darwin静态库87个FFI函数符号核查通过。
- 固定测试文档中的7类Mermaid图由随包helper实际生成，故意不支持的第8项返回诊断；这不等于整窗排版/交互验收。
- 日志：`/tmp/yu-precommit-workspace.log`、`/tmp/yu-precommit-clippy.log`、`/tmp/yu-precommit-release.log`、`/tmp/yu-precommit-native.log`，均为本地证据，不随Git提交。

本次按依赖与职责分批提交；各层提交组成同一交付系列，完整 Mac 验证针对系列最终状态，不声称每个中间提交都独立完成产品验收。

## 分批提交

1. `2b1bf076`：原生图表依赖源码、公式字体与许可证。
2. `8f841c90`：Rust内核、源码保持的结构编辑、资源服务和FFI。
3. `9a41e04b`：Mac写作功能、原生文档宿主与扩展内容集成。
4. `83c5b49e`：自动化脚本、原生测试与固定综合语料。
5. 本页所在文档提交：当前状态、验收缺口、测试入口及架构说明。

目标分支：`codex/mac-native-v3`。截图、隔离测试应用、构建产物不入库；随包必需的字体及其许可证已纳入依赖提交。GitHub推送结果以实际远端检查为准。
