# 第四组第一批：结构化粘贴拒绝的事务安全

基准：`509121800663138a3bbbbf64ad8f53463255ee5e`。本批仅增加回归测试、固定语料和真机验收标准，不改变现有支持边界，不代表第四组结项。

## 交付与当前验证状态

- Rust 回归入口：`crates/yu-editor/tests/html_paste_atomicity.rs`，六个测试入口；拒绝矩阵覆盖四种目标、四种 BOM/行尾组合、六种选区。测试必须在有仓库指定 Rust 工具链的环境执行。
- 共享语料：`crates/yu-editor/tests/fixtures/group4-paste/`。Rust 测试通过 `include_str!` 使用同一语料，真机副本由准备器生成。
- 准备器：`tools/prepare-group4-paste-checks.py`。只生成隔离测试文档、预期文件和哈希，不启动应用，也不判断产品通过。
- 准备器测试：`tools/test_prepare_group4_paste_checks.py`，覆盖精确字节、哈希、UTF-16 位置、不覆盖已有目录、缺失/损坏语料及状态标记。

编写本批时，本机 Runner 离线，开发容器无 Rust 工具链；未执行 Rust 测试、Clippy、Rust 格式检查、Mac 构建或实窗测试。准备器的六项 Python 测试已在 Linux 开发容器执行通过。后续验收以执行日志为准，不把这六项 Python 检查算成编辑器测试。

## 自动化检查

在待验收分支根目录运行，保存各命令的完整输出和退出码：

```sh
python3 -m unittest discover -s tools -p 'test_prepare_group4_paste_checks.py' -v
cargo fmt --all --check
cargo test --locked -p yu-editor --test html_paste_atomicity
cargo test --locked -p yu-editor --test html_table_edit
cargo test --locked -p yu-editor
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

新 Rust 文件应执行六个测试入口，不接受零测试退出成功。编译失败、依赖/工具链不可用、未运行、超时应分别记录，不计通过。格式检查失败时应在分支修复，并对新提交重新验证。

拒绝断言同时检查 `InvalidTablePaste`、源码、revision、完整 `Selections`、`HistoryStats`，连续重复三次，包括命令可用性查询。历史回放比较实际源码及重新绑定到当前 revision 的全部选区端点、方向、affinity、主选区和表格 slots，不能只看条目计数。

六种选区为：单光标、正向文字范围、反向文字范围、正向矩形、反向矩形，以及主选区不是第一项的两个非空选区。四种存储变体为：LF、BOM+LF、CRLF、BOM+CRLF。Rust 中的 BOM 是传入源码的 U+FEFF；Mac 文件 BOM 由存储层处理，真机 UTF-16 位置不计文件 BOM。

两类正向对照必须通过：拒绝跨组 rowspan 后的合法单行合并粘贴；公式/脚注引用替换为普通强调文本后，同一合并 payload 的 Markdown 表格转换。这用于排除“所有粘贴都失效但拒绝测试通过”。

## 真机准备

构建前记录完整提交 SHA 和 `git status --porcelain`。优先使用干净的独立工作区。按现有构建入口执行：

```sh
platform/macos/yu-shell-macos/build-app.sh --release
platform/macos/yu-shell-macos/run-self-checks.sh
python3 tools/prepare-group4-paste-checks.py artifacts/group4-paste-batch1
```

输出目录必须不存在。不得删除旧证据后复用同名目录；重试用新目录。准备器生成 28 个输入变体，均标为 `not_run`；`expected_paste_result` 是判据，不是执行结果。

记录 `.build/build-manifest.json` 中的应用和 helper 哈希，并独立计算实际运行的二进制哈希进行比较；相应文件位于 `platform/macos/yu-shell-macos/.build/`。使用现有脚本的隔离应用、状态目录及权限预检方式。记录系统版本、架构、输入源、主题、字号、缩放和窗口尺寸；不要操作日常文档或复用个人测试资料。

本批实窗必测浅/深色，以及 LF 无 BOM / CRLF 有 BOM。另两种编码由 Rust 全矩阵覆盖，可加做实窗但须单独记录覆盖。每个选区场景使用新的 `input.md` 副本或重新打开未修改输入，不能继承上一场景的历史。测试时关闭隔离实例自动保存，以便明确检查显式保存结果。

### 剪贴板路径不能替换

在单独的 Yu 文档窗口打开根输出目录的 `merged-payload.md`，使用 **Option+Shift 点击/拖选**选中整个合并格，再 Command-C。此操作应复制 Yu 的 version 2 结构化片段，`tableSource` 包含完整 `<table>`、`colspan='2'`、`rowspan='2'`（引号和属性顺序可能规范化），以及两行。

记录系统剪贴板的类型及结构化 payload，确认 `version == 2`、`fragments` 非空、`tableSource` 非空且仍是 2×2 的单一所有者。这里必须使用从 Yu 实际复制得到的 payload；普通浏览器 HTML、仅 `public.html`、纯文本或手写 Markdown 不能代替本路径。`DocumentTextView.pasteSourceFromPasteboard` 对这些表示有不同的分支和回退策略。

剪贴板准备失败应记作 setup failure，不能把“粘贴没有发生”当成安全拒绝通过。复制 donor 时不能修改目标文档；重复测试若改写了剪贴板，重新从 donor 复制。

### 建立既有历史

每个目标副本按以下方式建立相同的状态：

1. 打开 `input.md`，核对未编辑源码对应 `expected-0.md`（F0）。移动到文档最后一个字符之后，不新增换行。
2. 通过真实键盘输入 ` HISTORY-A-中文🙂`（A），确认源码为 `expected-a.md`（F1）。可分段输入或使用系统输入法提交，但须确保本步骤成为一个明确的撤销组；记录实际分组，不用时间间隔猜测。需要严格单组时可通过正常粘贴一次插入该完整普通文本，注明输入方式。
3. 显式移开再移回文档末尾，断开输入组。以同样方式插入 ` HISTORY-B-中文🙂`（B），确认源码为 `expected-b.md`（F2）。
4. 撤销 B，必须回到 F1；此时 A 可以撤销，B 可以重做。建立目标选区前先从 donor 重新复制结构化表格。

若 A/B 被拆成多个撤销组，重新准备该场景，不把后续额外按键解释成产品损坏。每个预期文件是精确字节参考；实时 AX 源码比较使用 `utf-8-sig` 解码以移除存储 BOM，磁盘比较必须保留 BOM 和行尾。

## 实窗场景与判据

以下每行均跑浅/深色和上述两种存储格式。选区必须用真实鼠标或键盘建立；AX/内部接口设定范围只算补充契约检查，不替代拖选验收。

| 编号 | 目标/操作 | 必须满足的结果 |
| --- | --- | --- |
| G4-PA-01 | `cross-groups`：光标放入“目标中文🙂”，结构化粘贴 | 跨两个 tbody 的 rowspan 被拒绝；不得合并行组、拆散传入合并格、插入纯文本或覆盖邻居。 |
| G4-PA-02 | `cross-head-body`：反向选中“目标中文🙂”，粘贴 | 跨 thead/tbody 拒绝；所选文字、方向、焦点和原始标签保持。 |
| G4-PA-03 | `cross-groups`：Option+Shift 从“矩形终点🙂”向“目标中文🙂”反向拖选，再粘贴 | 两个 colspan=2 所有者形成 2×2 选区；拒绝后选区不塌缩、不转成四个普通光标，邻居甲/乙不受影响。 |
| G4-PA-04 | `math-target`、`footnote-target`：分别以单光标及反向文字选区粘贴 | 转换拒绝；保留第三行第一列的公式/脚注引用、脚注定义、原始 Markdown 表格和历史。不得降为纯文字或生成失去语义的 HTML。 |
| G4-PA-05 | `math-target`、`footnote-target`：Option+Shift 从表头“矩形终点🙂”反向拖至“目标中文🙂”，再粘贴 | 拒绝后矩形仍覆盖原两格，不能缩为单光标；公式或脚注所在的未选中列不变。 |
| G4-PA-06 | 三个 `*-control` 目标，光标在“目标中文🙂”，粘贴同一 donor | 必须成功出现一个 2×2 合并格；两个 Markdown 对照转换成原生 HTML 表格，保留第一列强调文本和表格外源码。一次撤销回到粘贴前，重做恢复。 |
| G4-PA-07 | 完成 PA-03 拒绝后，从 `legal-one-row-payload.md` 真实复制单行合并格，保持原目标选区再粘贴 | 合法粘贴成功，只替换第一行的目标合并格，第二行与邻居不变。此时旧 B 的重做分支可以因成功编辑被清除；一次撤销恢复粘贴前源码及原矩形选区。 |
| G4-PA-08 | 完成拒绝和下面的完整历史回放后，保存、退出整个隔离进程、重开；再真实中文输入提交/取消 | 重开精确匹配保存文件，公式/脚注/跨度不变。中文提交可撤销，预编辑取消不改源码。重开后不要求恢复上一进程的撤销栈。 |

多选区的精确主选区/affinity 由 Rust 矩阵验证；若真机 AI 增加多光标契约检查，须记录全部范围和主选区，不得仅记录 `AXSelectedTextRange` 单数值。无现成外部观测字段时明确记为未观测，不编造内部状态。

### 每次拒绝后的强制检查

记录拒绝前后源码、可观察选区、菜单/错误表现和截图；连续粘贴三次，每次都应保持 F1。允许明确禁用或报告不支持，但不得报告成功并回退成另一种编辑。菜单禁用仅证明菜单门禁；同时检查实际 Command-V 路径。

在未做任何成功编辑的情况下，按 **重做 → 撤销 → 撤销 → 重做 → 重做**，对应源码必须依次为 **F2 → F1 → F0 → F1 → F2**。每步保存只读源码快照及可观察选区；任何一步需要额外按键、B 消失、出现粘贴内容或吞掉 A，均失败。最后 Command-S，磁盘 `input.md` 必须逐字节等于 `expected-b.md`：

```sh
cmp path/to/input.md path/to/expected-b.md
```

真正完整退出后确认原进程结束，再打开该文件；关闭/重开同一窗口不能充当完整进程重开。截图需实际复核，检查反向/合并格选择高亮、公式/脚注以及未编辑邻居；生成截图不等于视觉通过。

## 反馈与合入条件

每个结果至少包含：测试编号、完整提交 SHA、工作区是否有修改、应用/helper 哈希、系统/主题/编码、输入方式、选区方向、复现步骤、预期/实际、三次重复结果、源码快照和相关截图/日志路径。保留原始证据，不只交付总结。历史错误路径不能当成本轮证据。

缺少真实事件、代码执行、截图复核或构建身份时，对应维度维持 `not_run` / `blocked` / `needs_review`。准备器 manifest 只记录输入身份，构建和执行结果另存，不把输入 manifest 改成产品验收证明。

本批合入前须完成 Rust 编译及上述定向回归、格式/Clippy、工作区回归和对应真机项；失败反馈回到该分支修复。跨层级 HTML 列表、跨行组新语义、公式/脚注转换支持、Mermaid 边界及整组综合验收仍在原清单中，不能因本批通过而勾选完成。
