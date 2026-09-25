# Mac 第四组现状与验收入口

更新：2026-09-26。第四组尚未完成。本页是当前状态入口；逐轮实现细节保留在 [实现记录](mac-extended-document.md)，逐项证据见 [验收矩阵](mac-extended-document-acceptance.md)。

## 当前结论

原生公式、图表和扩展文档的生产链路已经接通，不是“还没有实现”。剩余工作集中在组合行为、明确支持边界，以及最新构建的统一实窗验收。后续以本页的任务清单跟踪，历史截图和历史测试通过结果不能直接证明新构建完成。

本批工作方式：直接修改 GitHub 仓库并更新 `main`，由测试真机拉取、打包和验收。GitHub CI 因额度不可用，本机 WebCodex Runner 暂不可用；不等待这两个通道，也不将 CI 修复插入功能开发。本次没有运行 Rust 编译/测试、格式检查、应用打包或实窗测试。2026-09-21 的桌面权限预检是历史记录，不代表当前连接或验收状态。

## 最近三批：代码状态与验证状态分开

| 批次 | 代码状态 | 本地检查与实窗状态 |
| --- | --- | --- |
| 第一批：粘贴拒绝原子性 | 已包含在基线 `0100eb8d`；[记录与标准](mac-group4-paste-atomicity.md) | 不将历史记录当作本次重跑；普通公式拒绝预期由第三批替换，脚注/跨行组拒绝保留 |
| 第二批：HTML 跨层级列表 | 已合入 `0100eb8de210dde93f9c03974b2cabe628300d81`，包含命令路由及选区模块；[批次记录](mac-group4-html-list-levels.md) | 本批不声称重跑列表实窗验收；综合构建仍需回归 |
| 第三批：公式表格转换 | 本页同批提交：逐单元格保留 TeX、有限 HTML 公式身份、原生组件投影、正文命中/编辑及成功/拒绝回归 | 编译、自动化测试和实窗验收均未执行；[本批标准与入口](mac-group4-table-math.md) |

第三批没有放开脚注、跨行组或全部公式语法。代码提交不等于功能验收通过；测试真机需记录实际拉取的提交 SHA、命令结果及构建哈希。

## 已实现的代码能力

| 功能 | 当前能力 |
| --- | --- |
| 原生公式 | MiTeX＋Typst 矢量输出；行内/块、分式、根式、矩阵、多行对齐、中文、编号引用；错误诊断与源码保留 |
| 原生图表 | Mermaid 七类基础图与扩展语料；ER 别名/属性；时序半箭头、Actor、创建/销毁；多项 Gantt 排程修复 |
| 辅助进程 | 随包、按需启动、版本校验、取消、空闲退出；旧简化公式模块已移除 |
| 扩展文档 | 脚注、目录、front matter、高亮、上下标；有限 HTML 段落、列表、表格、图片、对齐及折叠内容 |
| 结构编辑 | 合并表格布局/导航/结构修改/剪贴板往返；Markdown 表格接收合并内容；HTML 跨层级列表选区编辑；本批新增行内公式表格转换的源码保留及原生接入 |
| Mac 写作功能 | 图片插入/拖放/粘贴/尺寸、专注/打字机、拼写检查和设置；此前第三组已有本机验收记录 |

此前修复：`B-->>-A` 正确结束发送者 B 的激活状态，包含嵌套调用和实际激活条几何回归。中心连接 `()` 只有语义调研，没有实现，不能列入已完成项。

## 还没做完与后续顺序

1. **本批公式转换验证**：在测试真机完成新增/迁移回归、打包及浅深色实窗交互，核对中文/emoji、多公式、强调/链接、转义、未选中公式、BOM/CRLF、一次撤销重做、保存完全退出重开及拒绝原子性。
2. **脚注表格转换**：处理表格内引用与文档外定义的关联、重复引用和跳转，不能仅转换为上标数字。当前仍安全拒绝。
3. **HTML 跨行组粘贴**：明确合法组合与支持边界；暂不暗中合并 `<thead>/<tbody>`，也不拆散传入合并格冒充支持。第二批跨层级列表代码不再重复列作未实现，但仍纳入综合回归。
4. **图表边界**：中心连接 `()`、更多时序生命周期组合；Gantt 日期/时分秒格式、跨日和唤醒刷新。逐项给出支持范围与明确诊断，不以“能生成 SVG”代替语义正确性。
5. **冻结构建后的综合实窗验收和资源审计**：浅深色、真实中文输入、点击/拖选、撤销重做、保存完全退出重开；合并表格拖选/列宽和写作/图片回归。绑定同一应用/helper 哈希，汇总公式、图表、脚注、目录、HTML、错误恢复、取消与资源回收证据，完成后才能结项。

后续组别不计入第四组完成：第五组是 HTML/PDF/打印/图片/Pandoc 导出；第六组是 macOS 26 实机、VoiceOver、CI 环境、签名公证和分发更新。完整产品规划见 [路线图](mac-product-roadmap.md)。

## 本批测试入口

```sh
cargo test -p yu-editor --test html_paste_atomicity --test html_table_math
cargo test -p yu-markdown -p yu-editor
python3 -m unittest discover -s tools -p 'test_prepare_group4_paste_checks.py'
python3 tools/prepare-group4-paste-checks.py artifacts/group4-table-math-current
```

输出目录必须尚不存在。准备脚本现生成 36 份隔离样本，manifest 版本 2：普通公式和混排公式改为成功预期，非法公式、脚注和跨行组保留拒绝预期。全部状态初始为 `not_run`；生成文件不等于执行测试。操作细节及结果记录表见 [第三批验收标准](mac-group4-table-math.md)。

## 综合测试文档是哪个

**人工集中查看使用 [group4-smoke.md](../../platform/macos/yu-shell-macos/Fixtures/group4-smoke.md)**。它包含公式、七类图表、脚注/目录、扩展文字、HTML 列表/合并表格/details、本地图片及两项故意出错的语料。末尾未知公式和未知图表应显示诊断，这是预期，不是正常内容渲染失败。

请先复制此文件再编辑，以保留仓库基准。相对图片路径使用同级 `assets/yu-mark.png`，复制到其他目录时应同时保留 assets 目录。固定语料被提交不等于新构建验收通过。

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

## 历史验证记录：2026-09-21（不覆盖本次第三批提交）

以下保留原记录，不表示 2026-09-26 重新执行：

- 完整 Rust workspace：1444 项通过、5 项忽略、0 失败，包含 75 项 helper 库测试。
- 全工作区 Clippy、Rust 格式检查通过。
- CI 命令映射、依赖方向、FFI 头文件、条件依赖、rope 边界及坐标检查通过。
- Release 构建及应用审计通过；当时应用 SHA256：`1731a774376a3774ed957a4dd2074754ff061504464ae00153f0ad51e632d49f`。
- 当时应用 19 项原生自检通过；Darwin 静态库 87 个 FFI 函数符号核查通过。
- 固定测试文档中的 7 类 Mermaid 图由随包 helper 实际生成，故意不支持的第 8 项返回诊断；这不等于整窗排版/交互验收。
- 日志：`/tmp/yu-precommit-workspace.log`、`/tmp/yu-precommit-clippy.log`、`/tmp/yu-precommit-release.log`、`/tmp/yu-precommit-native.log`，均为当时本地证据，不随 Git 提交。

当时按依赖与职责分批提交；各层提交组成同一交付系列，完整 Mac 验证针对当时系列最终状态，不声称每个中间提交都独立完成产品验收。

## 历史分批提交：2026-09-21

1. `2b1bf076`：原生图表依赖源码、公式字体与许可证。
2. `8f841c90`：Rust 内核、源码保持的结构编辑、资源服务和 FFI。
3. `9a41e04b`：Mac 写作功能、原生文档宿主与扩展内容集成。
4. `83c5b49e`：自动化脚本、原生测试与固定综合语料。
5. 当时文档提交：当前状态、验收缺口、测试入口及架构说明。

以上提交已于 2026-09-21 快进合入并推送 `main`（代码提交 `9053612d`）。当时合并前核实：开发分支包含全部本地和远端分支的历史，无遗漏的独立提交。已删除本地 `codex/mac-native-v3`、`archive/v1-source-projection`、`s7-search`、`s7-multicursor`，以及远端同名开发/归档分支；历史提交仍完整保留在 `main`。

截图、隔离测试应用、构建产物不入库；随包必需的字体及其许可证已纳入依赖提交。合并不代表第四组已经完成。
