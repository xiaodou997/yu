# 第四组第五批：HTML 跨行组粘贴

更新：2026-09-26。开发基线：`db222c955d8f8c97c2538247f9b11dae7b00d199`。

本批只处理 HTML 表格跨行组粘贴的合法组合、边界检查、完整历史与选区恢复。公式和脚注沿用第三、第四批实现；不扩展 Mermaid、导出或 CI。代码交付到 GitHub `main`；核心检查在 `yu-workspace` 执行，应用打包和实窗验收另记。

## 结论与本批实际变化

**粘贴区域可以跨行组，单个传入合并格不能跨目标行组。** 目标已有的 `<thead>`、`<tbody>`、`<tfoot>` 及各自边界、属性和空组保留；不通过合并行组或拆分传入合并格来使粘贴成功。

此前局部覆盖路径已经会逐格拒绝真正跨界的跨度，部分合法跨组组合也已能工作。本批不是将所有跨组操作一律从拒绝改为放行，而是：

1. 在分割目标格、生成扩容副本之前，按目标连续行组的末行预检每个传入 owner。扩容只延续最后一个已有行所在的行组，不移动任何既有边界。核心错误 `HtmlTableError::CrossRowGroupSpan` 区分这类冲突；编辑命令仍沿用 `InvalidTablePaste`，没有新增 UI 提示文案。
2. 关闭带显式行组目标的“单元格全选 → 直接替换整个 table”旁路。选择全部单元格不是选择行组标签；这时也走覆盖计划，空行组同样保留。较小载荷只覆盖它自己的矩形，其余目标行/格保留。
3. HTML 结构粘贴与 Markdown 转换使用一致的选区边界：任意多光标不自动当作矩形；穿过格外标签的普通源码范围不能冒充单元格选择。
4. 重新解析结果后，除检查行列数，还检查目标行组的种类/相邻边界和每个传入 owner 的起点、行跨度、列跨度。语法合法但跨度被截短或位置偏移也不能提交。

无显式行组的裸 HTML 网格仍保留已有“全部单元格选择 → 完整网格替换”行为。直接替换原文属于普通源码编辑，不是本批的保组覆盖粘贴。

## 合法组合与边界

| 场景 | 本批规则 |
| --- | --- |
| 区域经过 `thead → tbody`、两个不同 `tbody`、`tbody → tfoot`，或连续三个行组 | 允许，前提是每个输入单元格的实际覆盖范围完全位于一个目标组内 |
| 每行各自的 `colspan`，区域整体跨组 | 允许，横向合并不跨行组边界 |
| 两个独立 `rowspan=2` 恰好分别落在两个两行组内 | 允许，两个 owner 和全部覆盖槽位保留 |
| 一个 `rowspan=2` 落在两个各一行的目标组上 | 拒绝；即使选中全部目标单元格也不得走旁路 |
| 输入与目标的行组名称/分段不同 | 以目标边界校验；覆盖复制的是单元格，不把输入的组容器移植到目标中 |
| 输入 `rowspan=0` 或声明大于其自身组内可见行数 | 沿用既有有限表格模型，将实际输入覆盖高度写成有限正数后校验；不让它在新目标中自动延长。不是拆散输入 owner，也不是新增 HTML 解析规则 |
| 从中间组开始，但必须向该组内插行才能容纳单个跨度 | 拒绝，不隐式向中间组插行或吞掉相邻组 |
| 向表格底部/右侧扩容 | 沿用既有扩容；新行加在最后一个已有行所在组中，尾部空组保持空；已有边界不变 |
| 未被覆盖的目标 `rowspan=0` | 保留其源码声明；若它所在末组扩容，其实际高度按原有规则延长 |
| 目标已有合并格与覆盖区域相交 | 仍可按既有覆盖规则分割目标 owner，原内容/id 只留在原左上格；不新增改变传入 owner 的规则 |
| 稀疏输入中的空槽位 | 沿用既有覆盖计划的空格补位，不新增稀疏表格语法规则 |
| 任意多选区、选区跨格外标签、非法结构、资源上限冲突 | 在提交前拒绝；已有源码、修订、全部选区、撤销/重做分支不变 |

HTML 的基础约束参考 [WHATWG 表格模型](https://html.spec.whatwg.org/multipage/tables.html#table-model)：一个单元格不能覆盖两个或更多行组；`rowspan=0` 的定义是延续到本组末行。Yu 的有限源码模型及剪贴板高度归一化是现有实现，不宣称本批实现了完整浏览器表格解析算法。

## 事务与回归

预检、准备副本和结果校验均不修改文档；合法覆盖仍只提交一项表格历史事务。一次撤销恢复此前全文、选区端点/方向/主选区和合并格槽位，一次重做恢复覆盖结果。成功的新编辑正常清除旧重做分支；被拒绝的操作和无变化覆盖都保留它。

新增 `crates/yu-editor/tests/html_table_row_groups.rs`：14 项集成回归。包含行组组合、横向/纵向 owner、零/超大声明的有限高度、末组/右侧扩容、空组保留、整表旁路、6 种选区形式、中文/emoji 与 BOM/CRLF、拒绝后合法跨组覆盖、无变化历史、剪贴板源码、公式和脚注、UTF-8 序列化重建，以及原生布局的单元格命中。跨度测试枚举 144 组目标行数/起点/输入高度组合，枚举次数不另算测试函数数。

第一批跨 `<thead>/<tbody>` 和多个 `<tbody>` 的真实跨度冲突测试仍保留，不改成成功断言。第三/四批公式、脚注测试和既有 HTML 编辑回归继续执行。UTF-8 重建、核心布局命中及 Rust 测试不替代应用实际保存、Command-click 或实窗回归。

```sh
cargo test -p yu-editor --test html_table_row_groups --test html_paste_atomicity --test html_table_edit --test html_table_math --test html_table_footnotes
cargo test -p yu-syntax -p yu-markdown -p yu-export -p yu-editor
cargo clippy -p yu-markdown -p yu-editor --all-targets -- -D warnings
cargo check -p yu-storage-ffi
python3 -m unittest discover -s tools -p 'test_prepare_group4_paste_checks.py'
```

本轮在 `yu-workspace` 实际执行的检查结果（不是 GitHub CI 结果）：

| 检查 | 结果 |
| --- | --- |
| `yu-syntax`、`yu-markdown`、`yu-export`、`yu-editor` 完整测试及文档测试 | 853 通过，0 失败；既有忽略项未改动 |
| 其中：本批行组 / 原子性 / 既有 HTML 表格编辑 / 公式 / 脚注 | 14 / 8 / 35 / 10 / 16 通过，均包含在总数中 |
| `yu-markdown`、`yu-editor` 全部 target 的 Clippy（`-D warnings`） | 通过，未放宽 lint |
| 本轮 3 个修改/新增 Rust 文件的 `rustfmt --check` | 通过，不声称全仓库格式检查 |
| Python 样本准备回归 | 9 通过 |
| `cargo check -p yu-storage-ffi` | 通过，不等于应用打包或 FFI 实窗验收 |
| 应用打包、真实剪贴板与浅深色实窗、保存完全退出重开 | 未执行 |

执行记录：完整 Rust 回归 `wc_job_6oBZGzwAzKdhEo1X`、Clippy `wc_job_zHFqHx5Q72LmoX41`、FFI 检查 `wc_job_6L_t2xjIfDlBRLdq` 均已结束且退出码为 0。记录绑定本次开发会话，不把旧批次日志当作本次结果。GitHub 提交完成后，以实际提交 SHA 和测试机应用/helper 哈希补充实窗证据。

## 固定样本：manifest 版本 4

```sh
python3 tools/prepare-group4-paste-checks.py artifacts/group4-row-groups-current
```

输出目录必须尚不存在。现有 52 份样本保留，新增 6 场景 × LF/CRLF × 无 BOM/有 BOM，共 76 份，其中 32 份为拒绝预期、44 份为成功预期。全部实窗 `status` 和 `native_test_status` 初始仍为 `not_run`。准备脚本不启动应用或执行验收。

**必须使用每个 manifest 项的 `payload` 字段，不能对所有场景一律使用原 `merged-payload.md`。** `required_selection=whole_table` 的两类场景必须选满目标单元格；其他项从“目标中文🙂”开始，或用“矩形终点🙂”建立矩形选择。普通跨格文本选择不能代替矩形。

| 场景前缀 | 目标行组 | 对应载荷 | 预期 |
| --- | --- | --- | --- |
| `row-groups-head-body` | 两行 head＋两行 body | `row-groups-merged-payload.md` | 两个 2×2 owner 各在一组内，成功 |
| `row-groups-body-body` | 两个独立两行 body | 同上 | 成功，不合并两个 body |
| `row-groups-body-foot` | 两行 body＋两行 foot | 同上 | 成功，保留 foot |
| `row-groups-span-conflict` | 两行 head＋两行 body | `row-groups-conflict-payload.md` | 三行 owner 跨界，拒绝 |
| `row-groups-whole-accept` | 一行 head＋空 body＋一行 body；全选 | `row-groups-rows-payload.md` | 两个独立横向合并格成功，组标签/id 不变 |
| `row-groups-whole-reject` | 同上；全选 | 原 `merged-payload.md` | 一个 2×2 owner 跨界，拒绝 |

## 真机验收标准（本轮不计通过）

在隔离副本上用当前应用执行，记录实际 `git rev-parse HEAD`、应用/helper 哈希、系统版本和浅/深色截图。

1. 先建立两项独立输入历史并撤销后一项，记录当前全文与完整选区；原重做分支须可用。
2. 按 manifest 指定的载荷，在应用中复制原生表格内容并粘贴。不能用直接输入 HTML 字符串代替剪贴板操作。成功场景检查目标组标签、组 id、空组、传入 owner 的合并形状、邻格和表格外字节；右侧的公式 `x^2` 和脚注定义仍应有效。
3. 正反向矩形选择分别操作。重点对两类 `whole` 场景选满单元格：合法覆盖成功，跨界的 2×2 owner 仍拒绝；不能因为全选就悄悄变成单一裸 table。
4. 一次撤销恢复全文与原选区，一次重做恢复覆盖结果。拒绝场景重复粘贴后立即重做原输入，证明分支未清除；随后执行合法跨组粘贴证明状态未污染。
5. 检查合并格内部点击、拖选、列宽、Tab 导航及继续输入的源码位置；末组扩容不移动中间边界，尾部空组不被意外填入内容。
6. 保存、完全退出、重开，检查 BOM/换行、公式与脚注、目标组边界、合并格和继续编辑。核心测试只能证明源码/模型合同，不能取代这些实窗证据。

后续功能继续按原顺序：Mermaid 时序图中心连接 `()` 与生命周期组合，再处理 Gantt 日期格式和跨日/唤醒刷新；完成后冻结构建进行第四组综合实窗与资源回收审计。第五组导出不提前插入。
