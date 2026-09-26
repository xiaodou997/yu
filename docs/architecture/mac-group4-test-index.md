# 第四组测试文档与执行入口索引

更新：2026-09-26。本页区分可直接在 Yu 打开的样本、验收说明和自动化入口；文档存在或输入生成不等于测试通过。当前冻结候选为标签 `group4-freeze-20260926-rc3`，源码 `6afb7a3dac785d2674236cff1fab65f3546fee39`；旧候选 `70b54992` 因表格公式未接入生产渲染而被拒绝，修复和重跑证据均保留。本轮执行结果见 [冻结构建与综合审计](mac-group4-freeze-audit.md)。

## 一、直接打开的测试文档

以下链接均指向仓库固定语料，先复制再测试，不在仓库原件上编辑。

| 文件 | 用途与预期 |
| --- | --- |
| [group4-smoke.md](../../platform/macos/yu-shell-macos/Fixtures/group4-smoke.md) | 第四组综合文档：公式、七类图表、脚注、目录、front matter、扩展文字、HTML 列表/合并表格/details 和图片。末尾两项故意出错，应显示诊断。复制时同时保留 `assets/yu-mark.png`。 |
| [group4-sequence-central.md](../../platform/macos/yu-shell-macos/Fixtures/group4-sequence-central.md) | 第六批中心连接与生命周期：前三段有效，后两段预期诊断；检查圆、箭头、编号、创建/销毁及嵌套激活。 |
| [group4-gantt-calendar.md](../../platform/macos/yu-shell-macos/Fixtures/group4-gantt-calendar.md) | 第七批日期时间与日历：前四段有效，后两段分别检查非法时刻和刻度超限；包含本地纯时间、跨年毫秒、12小时制、月份刻度。 |
| [render-embedded.md](../../platform/macos/yu-shell-macos/Fixtures/render-embedded.md) | 原有最小公式和流程图冒烟文档，不覆盖近七批全部组合。 |
| [native-parity-reference.md](../../platform/macos/yu-shell-macos/Fixtures/native-parity-reference.md) | 历史排版参考，不以逐像素复刻其他编辑器为本轮通过标准。 |

## 二、最近七批验收说明

这些 `.md` 是操作规则、边界和历史证据，不是全部可直接操作的样本。

| 批次 | 验收说明 | 当前适用范围 |
| --- | --- | --- |
| 第一批 | [mac-group4-paste-atomicity.md](mac-group4-paste-atomicity.md) | 结构粘贴拒绝、完整选区和历史原子性。最初的公式/脚注拒绝预期已分别由第三/四批替换；以最新样本 manifest v4 为准。 |
| 第二批 | [mac-group4-html-list-levels.md](mac-group4-html-list-levels.md) | HTML 跨层级缩进/退一级、父子选区、交叠和拒绝边界。文中旧补丁包应用流程和基准是历史记录，不适用于已包含实现的当前 main。 |
| 第三批 | [mac-group4-table-math.md](mac-group4-table-math.md) | Markdown 表格转换后的行内公式身份、TeX、点击编辑、一次撤销重做及保存。 |
| 第四批 | [mac-group4-table-footnotes.md](mac-group4-table-footnotes.md) | 表格脚注标签、文档外唯一确定定义、编号/重复引用/跳转及拒绝；未支持的标签/定义导入边界继续保留。 |
| 第五批 | [mac-group4-table-row-groups.md](mac-group4-table-row-groups.md) | 区域可跨行组、单个传入合并格不可跨目标组；保组全选、空组、扩容和完整历史。 |
| 第六批 | [mac-group4-sequence-central.md](mac-group4-sequence-central.md) | `()` 三位置、既有箭头组合、生命周期与激活边界、渲染与错误恢复。 |
| 第七批 | [mac-group4-gantt-calendar.md](mac-group4-gantt-calendar.md) | 日期时间/毫秒、轴刻度、日历上下文、跨日/唤醒刷新、输入与资源上限。 |

统一入口：[mac-group4-status.md](mac-group4-status.md)。总体验收矩阵：[mac-extended-document-acceptance.md](mac-extended-document-acceptance.md)。历史逐轮实现日志：[mac-extended-document.md](mac-extended-document.md)。旧记录不能证明新构建通过。

## 三、列表与表格的隔离输入

从仓库根目录执行；输出目录必须不存在，不覆盖旧证据：

```sh
python3 tools/prepare-group4-list-checks.py artifacts/group4-acceptance/list-inputs
python3 tools/prepare-group4-paste-checks.py artifacts/group4-acceptance/paste-inputs
```

| 材料 | 内容与使用方法 |
| --- | --- |
| [列表 `.case` 源语料](../../crates/yu-editor/tests/fixtures/group4-list) | 13份源语料展开为96份存储/位置变体；`.case` 含选区标记和前后期望，不能直接当产品文档。打开生成的 `input.md`，比较 `expected-0.md`、`expected-a.md`、`expected-b.md`、`expected-list-a.md`。 |
| [表格源语料目录](../../crates/yu-editor/tests/fixtures/group4-paste) | 最新准备器生成76份样本，44份成功预期、32份拒绝预期，覆盖四种BOM/换行组合；全部初始 `not_run`。 |
| `math-target.md`、`math-mixed-target.md` | 普通/混排行内公式目标；准备器还生成非法公式对照。 |
| `footnote-target.md`、`footnote-mixed-target.md` | 脚注与公式混排目标；准备器额外生成缺失/重复定义及非法节点对照。 |
| `cross-groups.md` | 单个传入 rowspan 跨目标组的拒绝基准，不代表所有跨组粘贴均拒绝。 |
| `row-groups-target.md`、`row-groups-whole-target.md` | 合法分组合并、整表单元格选择与空组保留。 |
| `merged-payload.md`、`row-groups-merged-payload.md`、`row-groups-conflict-payload.md`、`row-groups-rows-payload.md` | 不同传入形状；必须按每个 manifest 项的 `payload` 选择。`required_selection=whole_table` 必须选满目标单元格。 |

普通公式/合法脚注目前应转换成功；非法公式、无法唯一关联的脚注和真正跨界的单个合并格仍应拒绝。不要沿用第一批历史的28份样本数或第四批历史的52份样本数判断当前准备器。

## 四、自动化入口（不等于测试文档）

| 入口 | 检查层级 |
| --- | --- |
| [html_paste_atomicity.rs](../../crates/yu-editor/tests/html_paste_atomicity.rs)、[html_list_cross_level.rs](../../crates/yu-editor/tests/html_list_cross_level.rs) | 核心成功/拒绝、完整选区及历史。 |
| [html_table_math.rs](../../crates/yu-editor/tests/html_table_math.rs)、[html_table_footnotes.rs](../../crates/yu-editor/tests/html_table_footnotes.rs)、[html_table_row_groups.rs](../../crates/yu-editor/tests/html_table_row_groups.rs) | 近三批表格语义与核心布局。 |
| [sequence_connections.rs](../../tools/yu-document-renderer/tests/sequence_connections.rs)、[gantt_datetime.rs](../../tools/yu-document-renderer/tests/gantt_datetime.rs) | 生产 helper 解析/布局/向量语义。 |
| [client.rs](../../tools/yu-document-renderer/tests/client.rs) | 真 helper 请求、错误恢复、取消、回收；60秒空闲用例默认忽略，需明确执行。 |
| [RenderCalendarChecks.swift](../../platform/macos/yu-shell-macos/Tests/RenderCalendarChecks.swift) | 可注入日期与定时器的无窗口回归，不是真实跨午夜或睡眠通知。 |
| [run-self-checks.sh](../../platform/macos/yu-shell-macos/run-self-checks.sh) | 原生宿主/FFI自检；未带构建参数时使用既有包，先核对哈希。 |
| [run-embedded-checks.py](../../platform/macos/yu-shell-macos/run-embedded-checks.py) | 隔离应用、真实外部输入、截图、保存重开和资源场景；新增 `--recent-diagrams`、`--promotion-suite`、`--ime`、`--list-inputs`。旗标和二进制哈希分别记录，截图生成不等于复核通过。 |
| [run-writing-checks.py](../../platform/macos/yu-shell-macos/run-writing-checks.py) | 写作、图片及表格等相邻能力实窗回归。 |

检查命令、实际受测哈希和逐项结果统一记录在本轮冻结审计，不把一个层级的通过替代另一个层级。
