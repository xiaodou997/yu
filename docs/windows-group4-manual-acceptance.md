# Windows 第四组真机验收计划

计划日期：2026-10-01（Asia/Shanghai）。软件基线：`main@83e1bdb2`。
本计划的初始状态是**待执行**，不构成真机通过记录。

初次预检发现的 RTL 与启动阻塞，以及随后发现的可见布局/选区问题已修复；见
[2026-10-01 原生修复记录](windows-group4-native-fix-20261001.md)。
当前原生门禁通过，微软拼音第一至第三轮已由用户确认通过，其余人工验收待执行；初次失败证据保留于
[原生预检记录](windows-group4-preflight-20261001.md)。

## 2026-10-01 已执行人工记录

用户在本机屏幕、键盘鼠标会话中确认微软拼音第一轮通过：

| 项目 | 结果 | 证据范围 |
| --- | --- | --- |
| `zhongwen` 通过候选提交“中文” | PASS | 用户确认位置正确、只提交一次 |
| 输入 `ceshi` 后 Escape 完全取消 | PASS | 用户确认正文无残留 |
| Ctrl+Z / Ctrl+Y 撤销和重做提交文字 | PASS | 用户确认正常 |

该轮使用侧栏基础修复版 `3c40c048` 独立 exe，实测窗口 192 DPI。
此前疑似输入时退出在本轮未复现；不能据此确定此前退出原因。
本记录只覆盖上述基础步骤，不把 C01–C08 整套、其他 IME 或 DPI 清单记为 PASS。
随后样式版增加正文 HWND 页边距，见 [样式优化记录](windows-style-polish-20261001.md)；
新版复查结果见下面第二轮记录。

### 第二轮：新版页边距、选区替换与候选按键

用户确认 `round2-ime.md — Yu` 中安排的第二轮步骤全部正常。
版本基线为 `main@fc4558ecc9ca1cda6167f0a2fd3d15d527a69b36`，
独立 exe SHA256 为 `A9124CE31831078F0AA24DCC61318939CBDF9A8EAE1E9445D7C9D88432010DC3`，
窗口实测 192 DPI（200%）。

| 项目 | 结果 | 证据范围 |
| --- | --- | --- |
| 选中 `replace_me`，输入 `ceshi` 后 Escape | PASS | 用户确认原文字保留、无拼音残留 |
| 再次选中并以微软拼音候选提交“中文” | PASS | 用户确认仅替换一次，两侧 `before` / `after` 保留 |
| 新版页边距下候选框定位 | PASS | 用户确认靠近光标、无明显偏移；仅覆盖当前 200% |
| 候选中方向键及 Enter | PASS | 用户确认安排的选词/提交步骤正常；未覆盖全部抢键组合 |
| Ctrl+Z / Ctrl+Y | PASS | 用户确认恢复 `replace_me` / 重做“中文” |

上述为用户人工结果，程序存活、文件哈希和 Application 事件只作为辅助证据。
未保存的测试文档继续留在原窗口，不自动保存、关闭或替换。
第三轮执行情况见下表；实际系统 DPI 切换、其他输入法和完整 C01–C08 清单尚未全部通过。

### 第三轮：Unicode 邻接输入、搜索框与滚动/resize

用户确认安排的第三轮全部正常。使用与第二轮相同的 `fc4558ec` 产品代码和
`a9124ce3` 独立 exe，当前窗口仍实测 192 DPI。记录来源为用户人工确认，
窗口存活与 Application 事件读取作为辅助观察，不代替逐项实际操作。

| 项目 | 结果 | 证据范围 |
| --- | --- | --- |
| `A😀B𠀀C` 中 emoji/补充平面字符旁提交和取消 | PASS | 用户确认位置准确、字符完整、取消无残留，Undo/Redo 正常；C08-PY 的所安排子集 |
| 搜索框微软拼音提交和 Escape 取消 | PASS | 用户确认候选正常、无拼音残留、正文未被查询输入修改；UI05 当前 200% |
| 结束组合后搜索 Enter 跳转、Escape 返回正文 | PASS | 用户确认正常；UI04 的焦点/跳转子集 |
| 长文档滚动后打开候选、最大化/还原后再次输入 | PASS | 用户确认候选位置、滚动附近位置与正文显示正常，无重复提交；D03/D04 的当前缩放子集 |

第三轮未将全部 Unicode 编辑、鼠标拖选、系统缩放或其他 IME 记为通过。
第四轮准备进行真实系统缩放 200%→150%→125%→100%→200%，分别覆盖
字体/布局、候选提交/取消、滚动保持与越界拖选；各缩放下重新启动应用的路径单独记录。

### 第四轮首阶段：用户报告正常，150% DPI 证据待补

安排使用 `round4-dpi.md — Yu` 测试运行中 200%→150% 切换，检查滚动保持、
行首/行尾/下部候选提交和取消、resize 与越界拖选。用户回复“测试没有问题”。
这些操作的正常结果按用户报告记录；未收到切换前后顶部行号。

后续读取时测试 PID 15828 已不在运行，当前其他 Yu 窗口与屏幕读取均为
192 DPI（200%），因此未取得该次 150% 的实际 DPI 快照。
测试窗口关闭原因未知，Application 事件查询未找到该时间段匹配事件；
不能仅因进程已关闭判定崩溃，也不能据此证明全部期间没有异常。

150% 下的启动路径尚未执行，本阶段不记为完整 DPI 清单已通过。

### 本轮范围调整：用户要求跳过缩放测试

用户随后明确要求“这个缩放可以跳过了，下一步”。按此要求停止安排本轮的
150% 实测 DPI 补采集/启动、125%/100% 切换及恢复流程。
未执行部分记为 NOT_RUN，原因是用户要求跳过；已有人工操作反馈与当前 200%
验证记录保留，不把跳过记为通过或软件失败，也不继续要求用户修改系统缩放。
后续转入微软五笔、日文 Microsoft IME、韩文 IME 的兼容性验收。

### 其他输入法：用户报告基本项目正常

用户对安排的微软五笔候选提交/取消、日文假名与汉字转换、韩文音节组合与退格，
以及候选定位、选区替换、提交唯一性和 Undo/Redo 回复“这些测试没有问题”。
按该回复记录这些基本项目的人工反馈为 PASS，测试软件仍为 `fc4558ec` / `a9124ce3`。
测试进程 `other-ime.md` 仍在运行且响应正常。

该记录依据用户确认；未另行取得逐种输入法的模式、键序、兼容设置或 C01–C08
每项三轮的明细。此前配置快照只列出拼音，不作为其他输入法人工结果的替代证据。
本阶段不声称全部抢键/失焦边界已逐项完成，后续做保存重开、键鼠/剪贴板与连续编辑收尾。

### 最后一轮收尾：已准备，待人工结果

已打开独立测试窗口 `final-input.md — Yu`，沿用同一 `fc4558ec` / `a9124ce3`
软件基线。测试文件与日志位于忽略目录 `artifacts/windows-group4/20261001-final/`，
包括 Unicode/长文 fixture，以及取消、放弃、保存三个独立的关闭提示测试副本。
本轮只安排 Unicode 剪贴板与键鼠选择、包含中文/空格/emoji 文件名的另存与重开、
未保存提示三条分支，以及约 10 分钟/至少 20 次输入提交或取消的连续编辑。

当前状态为 AWAITING_USER，不预填 PASS。实际系统缩放按用户要求跳过，跨显示器
DPI 因单显示器不可测；收尾后按调整后的范围评估结项，继续保留上述证据限制。

## 本轮环境与范围

已经在当前机器读取到 Windows 11 专业版 10.0.26100、64 位、Intel Iris Xe。
显卡列表还包含 Microsoft Remote Display Adapter 和 OrayIddDriver；驱动列表不能证明
当前正在使用远程会话。测试者已确认本轮使用本机屏幕、键盘鼠标直接操作，只有一块显示器。

用户语言列表当前只有 `zh-Hans-CN`。执行前在输入法选择器中核实微软拼音、微软五笔是否可用，
并准备日文 Microsoft IME、韩文 Microsoft IME。缺失项记为 BLOCKED，不记为 PASS。
记录 Windows 完整 build/更新号、显卡驱动、分辨率、缩放、输入法名称及兼容模式设置；
语言列表不能代替实际输入法确认。

原计划单屏 100% / 125% / 150% 验收本轮已按用户要求跳过。
跨显示器不同 DPI 的迁移当前受单屏环境限制，仍作为未验收项记录。
当前 Yu 窗口实测为 200%（192 DPI），因此也保留 200% 的人工操作结果。
单屏切换缩放验证 DPI 变化的一部分路径，不能替代跨屏坐标、负坐标与显示器工作区测试。
Windows 第二、三组的 HWND / DirectWrite / D3D Present smoke 同轮执行。
图片、公式、Mermaid 与彩色 emoji 呈现属于第五组；本轮检查 emoji 的文本完整性与编辑行为。

## 执行顺序与时间

| 阶段 | 预算（准备好构建环境后） | 产出 |
| --- | --- | --- |
| A：原生构建与运行 smoke | 10–20 分钟，首次编译另计 | 原生测试日志、exe 哈希、HWND/Present 退出码 |
| B：基础键鼠、Unicode、剪贴板 | 20–30 分钟 | 保存文件、选择结果与异常录屏 |
| C：四种 IME 完整循环 | 40–60 分钟 | 每种 IME 的候选/提交/取消/抢键记录 |
| D：单屏 DPI 与窗口几何 | 20–30 分钟 | 三种缩放下截图、滚动位置与候选定位 |
| E：连续交互与报告 | 10–15 分钟 | 用例结果表与结项状态 |

建议准备完成后预留约 2 小时；首次工具链下载、语言组件安装不计入测试时间。
每个用例从样本的新副本开始。需要修改、取消、Undo 的用例不要相互继承状态。
候选词排序依赖输入法设置和历史，记录实际选择的候选文字，不要求固定候选序号。

## A：原生门禁与证据

在仓库根目录的 PowerShell 中执行。使用 Transcript 留存输出，
避免 Windows PowerShell 5.1 将重定向的 native stderr 当成终止错误。

```powershell
$run = Join-Path (Get-Location) ("artifacts/windows-group4/" + (Get-Date -Format "yyyyMMdd-HHmmss"))
New-Item -ItemType Directory -Force $run | Out-Null
git rev-parse HEAD | Set-Content (Join-Path $run "commit.txt")
git status --porcelain=v1 | Set-Content (Join-Path $run "worktree.txt")
Start-Transcript -Path (Join-Path $run "native.log")
try {
    & ./platform/windows/yu-shell-windows/run-self-checks.ps1
    if ($LASTEXITCODE -ne 0) { throw "Windows native self-check failed: $LASTEXITCODE" }
    cargo test -p yu-editor
    if ($LASTEXITCODE -ne 0) { throw "Editor regression failed: $LASTEXITCODE" }
} finally {
    Stop-Transcript
}
Get-FileHash ./target/debug/yu-shell-windows.exe -Algorithm SHA256 |
    Format-List | Out-File (Join-Path $run "exe-sha256.txt")
```

现有脚本执行 Windows 字体/render/shell 测试、native check，以及
`cargo run -p yu-shell-windows -- --window-self-check`。
从源码确认 self-check 会初始化真实 HWND、DirectWrite、D3D renderer、
完成首次 render/Present、初始化 TSF 并自动走 clean close。
通过记录须同时包含退出码 0 和对应执行日志；这不证明实际 IME 或持续交互通过。

若工具链、MSVC、SDK 或构建失败，先解决环境/实际失败，A 阶段保持 BLOCKED/FAIL。
不要用 cargo-xwin 成功记录替代 native 运行。
本轮已有软件门禁不必无条件重跑整个工作区；发现实际 bug 并修改代码后，再跑相关回归、
格式与 clippy，重新构建并记录新 commit/exe 哈希。

将测试样本复制到证据目录后打开，保护仓库原始样本：

```powershell
Copy-Item ./platform/windows/yu-shell-windows/Fixtures/group4-unicode.md (Join-Path $run "unicode-work.md")
Copy-Item ./platform/windows/yu-shell-windows/Fixtures/group4-scroll-crlf.md (Join-Path $run "scroll-work.md")
& ./target/debug/yu-shell-windows.exe (Join-Path $run "unicode-work.md")
```

| ID | 操作 | 通过判据 |
| --- | --- | --- |
| A01 | 执行上述 native self-check | 三个 Windows crate 测试、native check、真实窗口 smoke 均成功 |
| A02 | 执行 yu-editor 回归 | 无失败，保留本轮实际数量 |
| A03 | 手动打开样本；resize、最大化/还原、最小化/恢复；编辑一次 | 正文持续可见，窗口恢复后可编辑，无白屏/崩溃/错误对话框 |
| A04 | Save As 到含中文、空格、emoji 的新文件名；关闭再打开 | 文件身份/内容正确；untitled 首次保存走 Save As；dirty 关闭的 Cancel/Discard/Save 分支可用 |

A04 的 Save、Discard、Cancel 用三份独立临时副本；不要覆盖原始样本。

## B：基础输入、选择与 UTF-16 边界

先切为英文/直接输入状态，确保测试编辑器按键而不是输入法候选操作。

| ID | 操作 | 通过判据 |
| --- | --- | --- |
| B01 | 输入 ASCII、Enter、Tab；Backspace/Delete；左右、上下、Home/End、Ctrl+Home/End、PageUp/Down | 插入只发生一次；移动/删除位置与共享编辑语义一致；无重复换行/Tab |
| B02 | Ctrl+左右、Alt+左右、Shift+方向、Ctrl+Shift+左右、Shift+Home/End；Ctrl+A | 词移动/扩选可用，反向选区正确；Ctrl+A 覆盖完整文档 |
| B03 | Ctrl+C/X/V：Yu→记事本、记事本→Yu、Yu→Yu；覆盖选区粘贴多行 Unicode | 无乱码、丢字或重复；一次粘贴可撤销，剪切/粘贴替换位置正确 |
| B04 | Ctrl+Z、Ctrl+Y、Ctrl+Shift+Z；连续输入后移动光标再输入，撤销/重做 | 内容按编辑事务恢复；一次 IME 提交的历史另见 C05；不以“Undo 后 dirty 必须清空”为标准 |
| B05 | 在 `A😀B𠀀C` 两侧移动、扩选、Backspace/Delete；用 Win+. 直接插入 emoji 后保存/重开 | 不落在 surrogate 中间；不留下半个字符或 U+FFFD；源文件 UTF-8 可严格解码 |
| B06 | 对 `é`、`👩‍💻`、`🇨🇳`、`👍🏽` 重复移动、选择、删除、复制 | 按共享 grapheme 语义编辑，无损坏/孤立组成部分；颜色呈现不作为第四组失败判据 |
| B07 | 单击、Shift-click、正向/反向拖选；双击 ASCII、中文、emoji 后松开左键 | hit-test 与所见位置相符；双击结果遵循共享 UAX word boundary，松键不塌成 caret；不要求中文语言学分词 |
| B08 | 三击 CRLF 行、空行、长文档中的视觉折行 | 选择完整物理源行；软换行不当成新源行；CRLF 不拆成半个行结束符 |
| B09 | 长文档滚轮到中部/底部；向窗口上方、下方拖选，停留至少 3 秒，再反向并松开 | 自动滚动持续，选区跟随新 viewport 不落后一帧；松键停止；顶底边界无越界/抖动 |
| B10 | 在正文空白、行尾右侧、窗口四边附近定位/拖选；随后启动 IME | 无错误跳到 ACP 0、无非法位置/崩溃；候选位于最终 caret 附近 |

B10 是黑盒交互覆盖；不能据此声称直接验证了 `GetACPFromPoint` 所有 flags/HRESULT。
nearest 与窗口外边界的精确契约继续由已有单元测试覆盖。

B03 对未编辑区域和 Unicode 标量序列核对保存结果。CRLF/LF 转换若属于剪贴板输入，
单独记录，不将系统换行约定误判成 surrogate/ACP 损坏。
不要仅靠显示外观确认 Unicode 正确；检查保存文件并与期望字节/标量序列比较。

## C：真实 IME 矩阵

C01–C08 对下面四种输入法分别执行，记录为 `C01-PY` / `C01-WB` /
`C01-JP` / `C01-KR` 等完整 ID。每种至少做三轮完整 start→update→commit/cancel。

| 代号 | 输入法 | 建议文本/触发 |
| --- | --- | --- |
| PY | 微软拼音 | `zhongwen ceshi`，候选中选取“中文测试”或记录实际选择文字 |
| WB | 微软五笔 | 用测试者熟悉的五笔编码触发候选，选取至少两个汉字；记录编码与候选 |
| JP | 日文 Microsoft IME | Hiragana 模式输入 `nihongo`、`kyou wa ii tenki desu`，Space 转换并操作文节 |
| KR | 韩文 Microsoft IME | 韩文模式输入 `gksrmf`（双式键盘“한글”）；逐步观察音节组合与退格 |

日文/韩文用例由能核对文字的操作者执行；键盘布局不同就记录实际布局/键序。
韩文普通 Hangul 组合不一定弹候选窗；若配置支持 Hanja 转换，再用 Hanja 功能检验候选。
没有候选功能的具体模式记为 N/A 并说明原因，不将正常无候选判为失败。

| ID | 操作 | 通过判据 |
| --- | --- | --- |
| C01 | 空选区逐键输入，修改 preedit，Backspace，继续输入 | composition 更新正常，caret/下划线跟随；没有一份 preedit 外又一份正文 |
| C02 | 打开候选；Space、上下/左右、数字键或鼠标选候选；翻页（支持时） | 候选和文节操作符合该 IME；编辑器正文光标不意外移动/新增字符 |
| C03 | 通过 Enter/Space/候选选择完成提交，随后输入 `X` 并保存重开 | 提交文字仅出现一次，`X` 在正确位置；提交候选的 Enter 不额外插入正文换行 |
| C04 | 输入 preedit 后 Escape 直到整个 composition 取消；已有非空选区时重复 | 取消后原正文及选区替换前内容未受损；保存文件与本用例基线字节一致；Escape 不残留 preedit |
| C05 | 选择 `replace_me` 再开始 IME，先取消，再重新输入并提交；提交后 Undo/Redo | 取消保留原文；提交只替换选区；一次提交可完整撤销/重做，没有逐次 preedit 的历史 |
| C06 | composition/candidate 中按左右、上下、Enter、Escape，再结束 composition 后按同样键 | IME 活动时按键按该 IME 工作；结束后编辑器立即恢复导航、Enter 换行和正常输入 |
| C07 | composition 中点击其他位置、Alt+Tab、打开/取消文件对话框、切换中英/语言；返回继续输入 | 允许 IME 按自身规则提交或取消；无重复提交、丢失已提交文本、卡死或失焦后幽灵输入 |
| C08 | 在 `A😀B𠀀C` 前/中/后输入并取消/提交；中文与 emoji 交替输入 | ACP 映射准确；插入/替换位置正确，无 surrogate 拆分、U+FFFD 或选区错位 |

对于 C06/C07 的可疑行为，先在同一系统/输入法设置的记事本中做同样步骤，
记录系统 IME 的行为。不要硬性要求所有输入法用同一按键流程或失焦处理。
记事本用于区分系统行为，不取代 Yu 自身验收。

C04/C05 的保存比对必须在完全取消/提交后进行。
点击 Save 菜单本身可能改变焦点并触发 IME 提交，不能用“保存时是否看到 preedit”
证明 transient overlay 不写 canonical source。
“composition 不推进 Revision/不写 canonical”的内部契约继续以现有自动回归为证据；
本轮黑盒确认取消后字节不变、提交唯一、历史中没有逐次 preedit。

## D：单屏 DPI / caret / candidate 几何

每种缩放都记录分辨率、截图时间、窗口大小、当前输入法。
改变系统缩放前保存测试结果；若系统要求重新登录，记为一次新会话。
分别测试“应用保持运行时缩放变化”和“在该缩放下重启应用”。

| ID | 操作 | 通过判据 |
| --- | --- | --- |
| D01 | 100%、125%、150% 各启动一次；在行首/行尾、正文中部/可见底部打开候选 | caret 与文字对齐；候选锚点附近正确，无按旧 DPI 成比例偏移；底部候选可正常避让工作区 |
| D02 | 长文档滚动到第 060 行附近，记下顶部行号；保持应用运行改变缩放并 resize | 滚动状态保持并按新可视范围 clamp，不无故归零；命中位置和选区正确 |
| D03 | 开着候选/正在 composition 时 resize、移动窗口、最大化/还原，再继续选择与提交 | 候选位置及时更新或 IME 正常收起；重新触发后位置正确，文本只提交一次 |
| D04 | 滚动到文档底部、侧栏开关、resize；鼠标定位、双击及拖选后再打开候选 | 新 viewport/caret 几何一致，候选不跟随旧滚动位置；无一帧选区偏差 |
| D05 | 每种缩放进行一次窗口上下边缘越界拖选 | 自动滚动速度/命中与该 DPI 一致；停止操作后不持续滚动 |
| D06 | 两块不同 DPI 显示器间来回移动，含副屏负坐标；composition 开着时重复 | **当前单屏环境 BLOCKED**。补测须确认候选跟随新屏 caret、跨屏后滚动/命中与 Present 正常 |

输入法可能为了避让屏幕边缘把候选窗放在 caret 上方；这属于可接受布局。
失败判据是锚点明显错误、持续偏移、候选不可用或提交位置错。
D01–D05 至少用拼音完成整套；日文候选和韩文 preedit 在三种缩放下各做一次定位/提交。

## E：连续操作与结项

E01：用长文档连续 10 分钟进行滚动、拖选、切换 IME、输入、取消、提交、
Undo/Redo、保存、resize。至少 20 次 composition 完整循环，四种 IME 各有覆盖。
通过：没有崩溃、卡死、重复/丢失提交或累计候选位置漂移；保存后重开内容一致。

结果只使用 PASS / FAIL / BLOCKED / NOT_RUN / N/A；N/A 必须写原因。
报告保存在 `artifacts/windows-group4/<run>/`（仓库已忽略），不要把临时输出写回样本。

```text
baseline_commit:
exe_sha256:
tester:
date_and_timezone:
windows_build:
gpu_and_driver:
session: local
resolution_and_scale:
ime_and_compatibility_settings:

case_id | status | expected | actual | evidence_path | issue
A01     | NOT_RUN | ...      | ...    | ...           | ...
C01-PY  | NOT_RUN | ...      | ...    | ...           | ...
D06     | BLOCKED | 跨屏 DPI | 单显示器 | ...         | ...
```

异常至少记录：最短复现步骤、原文件/保存文件、输入法/模式/键序、
分辨率/缩放/窗口位置、预期与实际、候选框和 caret 同时可见的截图或短录屏。
取消比对失败须保留 baseline 和 actual 字节；不要只写“中文输入不正常”。

原始完整验收的结项规则：

- A–E 中适用用例执行通过，四种输入法没有 NOT_RUN/BLOCKED，关键异常修复并复测。
- 已提交文本丢失、重复提交、取消写入正文、surrogate 损坏、错误替换/不可恢复选区、
  崩溃/卡死、影响输入的候选定位错误都阻止结项。
- 本机单屏通过后记录“第四组单屏 Windows 真机验收通过，跨屏 DPI 待补”；
  依照原有验收范围，D06 补绿前不宣布完整第四组结项。
- D06 与全部适用项通过后，更新 `docs/windows-acceptance.md` 的实际记录，再正式进入第五组。
本轮用户已明确跳过系统缩放测试；本轮结果按调整后的范围记录，并附上未验收项。
不能以原计划强制用户继续已跳过项目，也不能将范围调整表述为完整 DPI 验收通过。
不能用计划、截图缺失或自动 self-check 退出码代替真实 IME 操作记录。

## 产品壳补测：2026-10-01 侧栏修复后

使用 `platform/windows/yu-shell-windows/Fixtures/group4-chrome.md` 的工作副本，
与 Unicode 样本放在同一个测试目录中。修复说明及已执行的定向消息检查见
[侧栏与滚动修复记录](windows-chrome-native-fix-20261001.md)。

| 编号 | 操作 | 通过条件 | 当前状态 |
| --- | --- | --- | --- |
| UI01 | 文件/大纲/搜索导航及状态栏 | 字体可读、选中清晰、控件不被背景遮挡 | 200% 截图已检查；物理操作待补 |
| UI02 | 单击/双击文件；脏文档切换时取消 | 单击仅选择；激活打开；取消保留原文档 | 定向消息打开通过；脏文档确认待补 |
| UI03 | 点击大纲标题、搜索结果 | 正文定位准确、目标进入可见区域、source/revision 不变 | 原生回归及可见截图通过；物理操作待补 |
| UI04 | Ctrl+F、搜索框输入/删除/Ctrl+Z、Enter、Escape、隐藏侧栏 | 搜索框操作不修改正文；焦点正确返回；隐藏后可继续正文输入 | 第三轮查询输入、Enter/Escape 人工通过；删除/Ctrl+Z/隐藏侧栏待补 |
| UI05 | 在搜索框用微软拼音 composition/commit/cancel | Enter/Escape 留给正在组合的输入法处理 | 第三轮当前 200% 人工通过 |
| UI06 | 100%/125%/150%/当前200%，浅色/深色 | 字体、行高、主题和导航清楚；滚动不变空白 | 原生字体96/192及深色渲染回归通过；其余实际缩放按用户要求本轮跳过，实际主题切换待补 |

测试窗口使用独立 exe 副本；用户开始输入后不关闭该窗口来解除编译锁。
任何疑似 IME 退出须保留测试进程 PID、退出时间、日志和 Application 事件，
将代理主动关闭与程序异常退出分开记录，不能将未复现的报告记为已修复。
