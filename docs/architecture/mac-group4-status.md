# Mac 第四组现状与验收入口

更新：2026-09-26。第四组尚未完成。本页是当前状态入口；逐轮实现细节保留在 [实现记录](mac-extended-document.md)，逐项证据见 [验收矩阵](mac-extended-document-acceptance.md)。

## 当前结论

原生公式、图表和扩展文档的生产链路已经接通，不是“还没有实现”。剩余工作集中在组合行为、明确支持边界，以及最新构建的统一实窗验收。后续以本页的任务清单跟踪，历史截图和历史测试通过结果不能直接证明新构建完成。

当前冻结构建：`group4-freeze-20260926-rc3`，源码 `6afb7a3dac785d2674236cff1fab65f3546fee39`，应用 SHA256 `ba4caa17724a5813a5c81d208311bd30a42683035773c8fec8187a3b52df539d`。本轮在 `yu-workspace` 实际完成 Release 打包／产物审计、1541项 Rust 回归、19项原生自检及16组浅深色／资源实窗矩阵；图像复核57张可见窗口。RC2 发现公式表格转换只保留源码却未进入生产公式索引，已修复并以 RC3 重跑。真实跨午夜／多日睡眠、长时压力及余下完整组合仍未验收，第四组不结项。精确哈希、失败与重跑证据见[冻结审计](mac-group4-freeze-audit.md)，最近全部材料见[测试文档索引](mac-group4-test-index.md)。GitHub CI 不可用且未修改配置；桌面预检本轮实际通过，不再以旧锁屏记录描述当前状态。

## 最近七批功能与第八批冻结审计：代码状态与验证状态分开

| 批次 | 代码状态 | 本地检查与实窗状态 |
| --- | --- | --- |
| 第一批：粘贴拒绝原子性 | 已包含在基线 `0100eb8d`；[记录与标准](mac-group4-paste-atomicity.md) | 普通公式、合法脚注拒绝预期分别由第三/四批替换；非法输入和单个合并格跨目标行组的拒绝回归仍保留，并纳入第五批重跑 |
| 第二批：HTML 跨层级列表 | 已合入 `0100eb8d`；[批次记录](mac-group4-html-list-levels.md) | RC3 已执行独立／单元格8个单范围固定场景，浅深色各通过；多选区／反向实鼠和96份全排列未全跑 |
| 第三批：公式表格转换 | 初版 `452e89f7`；本轮 `a8ff7794` 修复 HTML 公式未注册生产 EquationIndex 的漏接 | RC3 的12项核心回归，以及真实合并复制粘贴、公式鼠标命中／正文编辑、一次历史恢复和重开通过；[记录](mac-group4-table-math.md) |
| 第四批：脚注表格转换 | 已合入 `db222c95`，原标签、全文编号与外部定义关联；[记录](mac-group4-table-footnotes.md) | RC3 核心重跑；浅深色真实转换、Command-click 定义、历史和保存通过；并非所有合法／拒绝排列已跑完 |
| 第五批：HTML 跨行组粘贴 | 已合入 `843eddec`；[记录](mac-group4-table-row-groups.md) | RC3 核心重跑；浅深色真实 donor 拖选、合法跨组及真实跨界拒绝通过，拒绝后旧重做分支保留；全选／空组／列宽的全窗口排列仍待补 |
| 第六批：时序中心连接与生命周期 | 已合入 `f891e971`；[记录](mac-group4-sequence-central.md) | RC3 核心重跑，三种固定组合的浅深色真实标签编辑／历史／保存重开通过；长图截图仅证明可见区域 |
| 第七批：Gantt 日期时间与日历刷新 | 已合入 `29643df0`；[边界与验收](mac-group4-gantt-calendar.md) | RC3 核心／原生日历重跑，四种日期图的浅深色编辑／历史／保存重开通过；真实跨午夜／睡眠／时区事件未执行 |
| 第八批：冻结与综合审计 | RC3 源码 `6afb7a3d`；`main` 后续只补测试与证据，不移动标签 | 1541 Rust／19原生通过；16组实窗、164条脚本检查记录通过；有限生命周期／取消回收通过；完整结项缺口见[审计](mac-group4-freeze-audit.md) |

第四批接通有唯一文档外定义的脚注转换；缺失/重复定义、解析器分歧及非法 HTML 仍安全拒绝。第五批明确“区域可跨组、单个传入合并格不可跨目标组”；单元格全选也不能删除目标行组标签来绕过检查。没有扩大全部公式语法。本轮已绑定实际源码、应用／helper 哈希与代表性实窗证据，但未执行的系统时间／长时压力／完整排列不因这些通过而自动结项。

## 已实现的代码能力

| 功能 | 当前能力 |
| --- | --- |
| 原生公式 | MiTeX＋Typst 矢量输出；行内/块、分式、根式、矩阵、多行对齐、中文、编号引用；错误诊断与源码保留 |
| 原生图表 | Mermaid 七类基础图与扩展语料；ER 别名/属性；时序半箭头、中心连接、Actor、创建/销毁及嵌套激活；Gantt 数字日期时间/毫秒、12小时制、时间/月刻度与本地日期刷新 |
| 辅助进程 | 随包、按需启动、版本校验、取消、空闲退出；旧简化公式模块已移除 |
| 扩展文档 | 脚注、目录、front matter、高亮、上下标；有限 HTML 段落、列表、表格、图片、对齐及折叠内容 |
| 结构编辑 | 合并表格布局/导航/结构修改/剪贴板往返；Markdown 表格接收合并内容；HTML 跨层级列表选区编辑；公式/脚注表格转换；保留目标行组的合法跨组覆盖及明确跨度拒绝 |
| Mac 写作功能 | 图片插入/拖放/粘贴/尺寸、专注/打字机、拼写检查和设置；此前第三组已有本机验收记录 |

`B-->>-A` 继续结束发送者 B 的激活。第六批已实现中心连接 `()`，并修正独立激活指令与自调用返回段的几何锚点；中心连接不改变激活深度，非法结束会诊断。实现/核心测试不等于应用实窗验收，完整支持边界见第六批记录。

## 还没做完与后续顺序

1. **余下表格／列表窗口排列**：RC3 的真实复制粘贴、公式点击编辑、脚注跳转、合法跨组与拒绝原子性已通过；继续补全全选／空组、更多编码／反向／多选区及合并表格列宽实鼠操作。准备器96／76份不能一键标成全通过。
2. **脚注转换支持边界**：已支持唯一文档外定义、重复引用和跳转；字面实体标签的转换器分歧仍明确拒绝，不自动导入/重命名外来定义。下一批不再重复开发已接通的基本链路。
3. **整份综合文档与相邻流程**：专项浅深色测试已经重跑；`group4-smoke.md` 的目录／details跳转、屏外长页内容及全部写作流程仍未在同一综合场景中重跑，不能以独立专项代替。
4. **日历真实环境验收**：中心连接与 Gantt 静态日期时间图已完成代表性实窗；尚缺无输入跨午夜、多日睡眠／时区／时钟变化及隐藏恢复的系统通知组合。注入日期测试不替代这些；不支持的格式仍按[第七批](mac-group4-gantt-calendar.md)拒绝。
5. **长期资源与最终签收**：RC3 已真实验证普通文档不启动helper、空闲退出／回收、编辑重启、取消旧响应与应用退出释放，也跑过真实拼音输入。仍缺长期高频／多文档压力，不以50组短时内存样本宣称无泄漏。保留冻结标签，余下证据完成后再结项。

后续组别不计入第四组完成：第五组是 HTML/PDF/打印/图片/Pandoc 导出；第六组是 macOS 26 实机、VoiceOver、CI 环境、签名公证和分发更新。完整产品规划见 [路线图](mac-product-roadmap.md)。

## 第七批 Gantt 与日历测试入口

```sh
cargo test -p yu-document-renderer --locked
cargo test -p yu-storage-ffi --lib reference_day
cargo clippy -p yu-document-renderer -p yu-storage-ffi --all-targets -- -D warnings
```

真机使用 [group4-gantt-calendar.md](../../platform/macos/yu-shell-macos/Fixtures/group4-gantt-calendar.md) 的隔离副本：前四段有效，后两段为预期诊断。独立 Swift 日历测试已纳入自检脚本，第八批已执行整份当前构建原生自检；真实系统日期通知仍未验证。纯时间图使用宿主本地日期，显式日期不随今日重排；民用时间轴不是带时区的绝对时刻模型。精确结果、支持边界与操作步骤见[第七批记录](mac-group4-gantt-calendar.md)。当前 RC3 已冻结并完成本轮记录的实窗／有限资源审计；继续补审计缺口，不提前进入第五组导出。

## 第六批时序测试入口

```sh
cargo test -p yu-document-renderer
cargo clippy -p yu-document-renderer --all-targets -- -D warnings
```

真机使用 [group4-sequence-central.md](../../platform/macos/yu-shell-macos/Fixtures/group4-sequence-central.md) 的隔离副本：前三段为合法图，后两段为预期诊断。逐项标准及本轮实际结果见[第六批记录](mac-group4-sequence-central.md)。

## 前五批表格回归入口

```sh
cargo test -p yu-editor --test html_table_row_groups --test html_paste_atomicity --test html_table_math --test html_table_footnotes --test html_table_edit
cargo test -p yu-syntax -p yu-markdown -p yu-export -p yu-editor
python3 -m unittest discover -s tools -p 'test_prepare_group4_paste_checks.py'
python3 tools/prepare-group4-paste-checks.py artifacts/group4-row-groups-current
```

输出目录必须尚不存在。准备脚本现生成 76 份隔离样本，manifest 版本 4：保留前四批 52 份，新增跨行组 24 份。必须使用每项 `payload` 指定的剪贴板样本，`required_selection=whole_table` 的项必须选满单元格。合法跨组与真正跨界分别为成功/拒绝预期；总计 32 份拒绝预期。全部实窗状态初始为 `not_run`；生成文件不等于执行测试。核心检查、操作细节和边界见[第五批验收标准](mac-group4-table-row-groups.md)。

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

## 历史验证记录：2026-09-21（不覆盖本轮提交）

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
