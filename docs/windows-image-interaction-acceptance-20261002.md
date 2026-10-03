# Windows 图片交互实窗验收（2026-10-02）

## 结论和环境

**本轮不能判定为完整通过。** 大部分本地图片主链路已实际验收；七处缺陷已修复并复测。两条“替换图片”入口仍崩溃，浮动条仍会遮挡内容，远程图片没有真实像素。100% / 125% / 150% 系统缩放的实窗测试受当前远程会话限制，未完成。

- 仓库 `D:\QTDownLoads\yu`，分支 main；开始时工作区干净。
- 已 fetch / pull，结束前再次 fetch：HEAD 与 origin/main 均为 `c5878fb62a6a3a2f60c6b29793f658b57dd5db93`。`git merge-base --is-ancestor c5878fb6 HEAD` 返回 0。
- 实测日期 2026-10-02，Asia/Shanghai；Windows 11 Pro x64，10.0.26100。
- **实际窗口 DPI=192，即 200% Windows 缩放。** 不把图片预设百分比当成系统缩放。
- 实际编译启动 `target\debug\yu-shell-windows.exe`，打开磁盘 Markdown；最终构建 SHA256 见 [tested-binary.txt](../artifacts/image-acceptance/tested-binary.txt)。未创建分支或提交，修复保留在工作区。
- 鼠标点击 / 拖动通过 Win32 实际输入，键盘通过 SendKeys / 粘贴，原生控件也是实际点击。不是直接调用内部编辑命令来替代实窗验收。
- 截图来自真实桌面窗口；HWND 读取字段 / DPI / 矩形，磁盘文件核对保存内容，外部程序验证复制 / 打开 / 定位。
- 普通窗口包括 2100×1700、1800×1600，窄窗口 900×1100（物理像素），以及最大化 / 还原。

## 使用的 Markdown

完整初始文件：[original.md](../artifacts/image-acceptance/original.md)。最终文件：[acceptance.md](../artifacts/image-acceptance/acceptance.md)。实际文件有 100 段长正文和末尾标记，约 8900 字符，可长距离滚动。图片及边界片段如下：

```markdown
# 图片交互实窗验收

TEXT START 普通正文点击、拖选、double word、中文输入。

![Local PNG](assets/local.png)

![Local JPG](assets/local.jpg)

![中文空格](<assets/中文 空格/测试 图片.png>)

![Remote](https://www.python.org/static/community_logos/python-logo.png)

![Adjacent A](assets/local.png)
![Adjacent B](assets/local.jpg)

- 列表附近

![Near list](assets/local.png)

> 引用附近

![Near quote](assets/local.jpg)

| 列一 | 列二 |
| --- | --- |
| 表格 | 图片旁 |

![Wide](assets/wide.png)

![Small](assets/small.png)
```

实际生成的图片：PNG 320×160、JPG 240×120、中文空格路径 PNG 360×180、替换候选 PNG 400×200、超宽 PNG 1800×240、小 PNG 24×16；不同底色和图片内标识便于确认刷新。

最终第一张图片为：

```html
<img src="assets/local.png" alt="实窗 Alt 中文" width="150" height="75">
```

尺寸修改使用 HTML img 保存宽高，仍属于 Markdown 文档源码。结束时比对：除第一张图片这一行外，其余内容与初始文档一致，未误改正文或邻近图片。

## 通过

### 单击、浮动条、退出选中

- 实际单击 PNG / JPG / 中文空格路径图片，保持渲染，显示蓝色描边及浮动条，不切回普通 Markdown 源码模式。
- 浮动条显示正确源码、Alt、地址，以及替换、图片大小、更多入口。替换与更多目前都显示省略号，替换失败另列。
- 点击正文、Esc、继续键盘输入可退出选中。Alt 输入框内 Esc 原有问题已修复，实窗复测控件确实隐藏。
- 截图：[final-toolbar-200.png](../artifacts/image-acceptance/final-toolbar-200.png)、[navigation-escape-field.png](../artifacts/image-acceptance/navigation-escape-field.png)、[navigation-type-exit.png](../artifacts/image-acceptance/navigation-type-exit.png)。

### 滚动和窗口变化：位置跟随部分

- 实际上下滚动、部分滚出正文 viewport、完全滚出、拖动滚动条、缩窄窗口、最大化 / 还原。
- 修复后边框随图像位置移动，越出正文的边缘隐藏 / 裁剪；完全离屏不留悬空控件，窄窗口不再 panic。
- 证据：[navigation-partial-scroll.png](../artifacts/image-acceptance/navigation-partial-scroll.png)、[navigation-offscreen.png](../artifacts/image-acceptance/navigation-offscreen.png)、[navigation-scrollbar-drag.png](../artifacts/image-acceptance/navigation-scrollbar-drag.png)、[navigation-narrow.png](../artifacts/image-acceptance/navigation-narrow.png)、[navigation-maximized.png](../artifacts/image-acceptance/navigation-maximized.png)、[navigation-restored.png](../artifacts/image-acceptance/navigation-restored.png)。
- **不能据此判定“不遮挡正文”通过**，遮挡问题仍在；未做高帧率视频分析，不保证每个瞬间完全无闪烁。

### Alt / 地址、历史和保存重开

- 真正通过浮动输入框修改 Alt 为 `实窗 Alt 中文`，失焦提交；检查源码、一次 Undo、Redo、保存退出程序并重开。
- 地址改为 `assets/中文 空格/测试 图片.png`，失焦后橙色图片立即变为绿色，Alt 和 150×75 宽高保留；HTML 地址按程序规则百分号编码，重开可正确解码文件。
- 地址同样通过 Undo / Redo / 保存退出重开；最后恢复 `assets/local.png`。
- 证据：[field-alt-unicode.png](../artifacts/image-acceptance/field-alt-unicode.png)、[field-destination-unicode.png](../artifacts/image-acceptance/field-destination-unicode.png)；源码及持久化断言见 [results-final.json](../artifacts/image-acceptance/results-final.json)。

### 双击图片属性

真实双击图片直接打开“图片属性”，不走普通文字双击；检查地址、Alt、宽、高、锁定纵横比。每个用例先恢复原始无宽高图片：

| 输入 | 锁定比例 | 应用结果（逻辑尺寸） | 判定 |
| --- | --- | --- | --- |
| 宽高空 | 关闭 | 不写 width / height | 通过 |
| 仅宽 180 | 关闭 | 只写 width，渲染 180×90 | 通过 |
| 仅高 70 | 关闭 | 只写 height，渲染 140×70 | 通过 |
| 宽 210、高 90 | 关闭 | 保存并渲染 210×90 | 通过 |
| 改宽 200 | 开启 | 自动补高 100，保存 200×100 | 通过 |
| 改高 75 | 开启 | 自动补宽 150，保存 150×75 | 通过 |

发生修改的用例逐一执行应用、Undo / Redo、保存关闭重开。原本为空的用例未制造虚假历史。取消 / Esc 实测文档不变，模态窗口关闭，主窗恢复可操作。

证据：[properties-width-draft.png](../artifacts/image-acceptance/properties-width-draft.png)、[properties-both-applied.png](../artifacts/image-acceptance/properties-both-applied.png)、[properties-locked-width-draft.png](../artifacts/image-acceptance/properties-locked-width-draft.png)、[properties-locked-height-applied.png](../artifacts/image-acceptance/properties-locked-height-applied.png)。证据目录有每组 draft / applied / undo / redo / reopen 截图。

### 图片大小十种操作

逐项实际打开菜单选择；每项检查源码、渲染尺寸、单次 Undo、Redo、保存关闭重开。连续切换再回到原始大小，没有发现累积比例漂移。

| 菜单项 | 源码宽高（逻辑尺寸） | 实测外边界（200% 物理像素） |
| --- | --- | --- |
| 25% | 80×40 | 160×80 |
| 33% | 106×53 | 212×106 |
| 50% | 160×80 | 320×160 |
| 67% | 214×107 | 428×214 |
| 80% | 256×128 | 512×256 |
| 100% | 320×160 | 640×320 |
| 150% | 480×240 | 960×480 |
| 200% | 640×320 | 1280×640 |
| 原始大小 | 清除宽高 | 640×320 |
| 适应正文宽度 | width=749，不写 height | 1498×749 |

权威几何记录：[sizes-geometry-final.json](../artifacts/image-acceptance/sizes-geometry-final.json)，十项均匹配。截图：[size-33-200.png](../artifacts/image-acceptance/size-33-200.png)、[size-200-200.png](../artifacts/image-acceptance/size-200-200.png)、[size-fit-200.png](../artifacts/image-acceptance/size-fit-200.png)。

早期脚本的 rendered 字段读的是内嵌描边之间的空隙，少了 8 物理像素；最终 geometry 文件使用同一实窗的外侧 HWND 边界重新计算，不把内部空隙误当图像尺寸。脚本校准日志保留，不充当产品失败或最终通过证据。

### 本地右键菜单及外部应用

- 完整菜单实际检查替换、打开、在资源管理器显示、复制图片、复制地址、大小、属性、源码；入口存在不代表替换通过。
- 打开图片触发 Windows 应用选择确认，选择标为默认应用的 PicView，并选“仅一次”，不改系统默认关联；实际 PicView 显示 local.png / 320×160。证据：[open-image-native-chooser.png](../artifacts/image-acceptance/open-image-native-chooser.png)、[open-image-default-viewer.png](../artifacts/image-acceptance/open-image-default-viewer.png)。
- Explorer 定位修复后，实际 SelectedItems.Path 精确等于本地 PNG 和中文空格 PNG 的完整路径，不只是打开文件夹。证据：[explorer-selection-final.png](../artifacts/image-acceptance/explorer-selection-final.png)、[chinese-space-context.png](../artifacts/image-acceptance/chinese-space-context.png)。
- 复制图片后剪贴板含图像 / DIB，位图为原图 320×160。环境未装画图，改用已安装 Photoshop 2025：真正新建剪贴板尺寸画布，Ctrl+V 得到橙色 PNG 图层和 320×160 尺寸，之后不保存关闭。证据：[clipboard-paste-final.png](../artifacts/image-acceptance/clipboard-paste-final.png)。
- 复制地址实际得到 `assets/local.png`。
- 编辑源码：先让 Alt 框持有焦点，再右键第一张图进入源码并 Ctrl+C，得到完整精确 HTML 标签；连续图片 B 得到严格的 `![Adjacent B](assets/local.jpg)`，没有命中 A。证据：[source-first-focused-final.png](../artifacts/image-acceptance/source-first-focused-final.png)、[source-adjacent-b-exact.png](../artifacts/image-acceptance/source-adjacent-b-exact.png)。

### 远程条目的通用菜单

实际右键没有本地专属打开 / Explorer / 图像复制；通用地址、属性等存在。复制得到原始 HTTPS URL，属性可打开取消。证据：[remote-context-verified.png](../artifacts/image-acceptance/remote-context-verified.png)、[remote-properties.png](../artifacts/image-acceptance/remote-properties.png)。**仅这部分通过，真实远程图像加载不通过。**

### 边界与普通编辑回归

- 连续 A / B 图片、列表附近、引用附近分别实际选中，浮动条源码命中正确图片；表格附近也操作，但有遮挡缺陷。
- 超宽图按正文约束，小图可选中并双击属性取消；中文空格文件名、900×1100 窄窗口实际操作。证据：[boundary-adjacent-b-selected.png](../artifacts/image-acceptance/boundary-adjacent-b-selected.png)、[boundary-near-list-selected.png](../artifacts/image-acceptance/boundary-near-list-selected.png)、[boundary-near-quote-selected.png](../artifacts/image-acceptance/boundary-near-quote-selected.png)、[boundary-wide-narrow.png](../artifacts/image-acceptance/boundary-wide-narrow.png)、[boundary-small-properties.png](../artifacts/image-acceptance/boundary-small-properties.png)。
- 普通正文点击定位、拖选、双击 TEXT 选词并复制检查、滚动条、英文输入、Undo / Redo、保存实际测试。证据：[regression-double-word.png](../artifacts/image-acceptance/regression-double-word.png)、[regression-drag-text.png](../artifacts/image-acceptance/regression-drag-text.png)。
- 真正微软拼音输入 zhongwen，显示预编辑候选，空格上屏“中文”，保存源码确认，Undo 精确恢复、Redo 重做，最后撤销测试输入。不是用粘贴汉字代替 IME。证据：[ime-pinyin-preedit.png](../artifacts/image-acceptance/ime-pinyin-preedit.png)、[ime-committed.png](../artifacts/image-acceptance/ime-committed.png)。

## 问题

### P1：替换两条入口都在原生选择器阶段崩溃（未修复）

- **缩放 / Markdown：** 200%，DPI 192；本文最终第一张 HTML 标签，初始 `![Local PNG](assets/local.png)` 也复现。替换候选为 assets/replacement.png（400×200）。
- **步骤 A：** 启动并打开 acceptance.md → 单击第一张橙色图 → 点击浮动条替换按钮。
- **步骤 B：** 重启同一文档 → 右键第一张图 → 选择“替换图片”。
- **预期：** 原生 Windows 选择器打开；选候选后立即刷新、Alt / 宽高不变；一次 Undo 恢复，Redo 重做，保存重开保留。
- **实际：** 两条入口都在显示选择器阶段直接退出，没有可操作的选择器。因此替换刷新、保留设置、替换历史和持久化全部未通过。
- **截图：** [浮动入口点击前](../artifacts/image-acceptance/final-before-replace.png)、[右键入口点击前](../artifacts/image-acceptance/final-right-replace-before.png)。进程退出没有后续图片 UI，错误结果保存为 [Windows Application 错误事件](../artifacts/image-acceptance/windows-crash-events.txt)，不伪造崩溃后界面。
- **错误：** comdlg32.dll 10.0.26100.4484，异常 0xc0000005，偏移 0x79048；最终两次时间为 2026-10-02 22:12:57、22:15:44，Asia/Shanghai。
- **定位：** 临时跟踪确认 IFileOpenDialog 已创建 / 配置，崩溃在 IModalWindow::Show 期间。普通 Ctrl+O 也复现，同机 .NET 原生 OpenFileDialog 对照能显示。过滤器存储实验未解决，实验修改撤回。根因尚未确定，不能只凭 DLL 名称认定是 Windows 本身的问题。
- **后续：** 需要崩溃转储 / 调用栈定位原生选择器和 COM 调用，然后重测完整替换链路。改地址成功不能代替替换验收。

### P2：浮动条遮挡正文 / 表格（未修复）

- **缩放 / Markdown：** 200%，DPI 192；`![Near quote](assets/local.jpg)` 后紧接本文表格；窄窗口另用第一张图及上方标题 / 正文。
- **步骤：** 2100×1700 窗口选中 Near quote → 下滚使图片和表格靠近可视区顶部；或选中第一张图后缩到 900×1100。
- **预期：** 浮动条跟随图片且不遮正文 / 表格 / 标题。
- **实际：** 跟随基本正确，但工具条覆盖表格表头；窄窗口覆盖标题 / 正文，普通窗口也能覆盖下一张图。
- **截图：** [boundary-table-wide-small.png](../artifacts/image-acceptance/boundary-table-wide-small.png)、[navigation-narrow.png](../artifacts/image-acceptance/navigation-narrow.png)、[final-toolbar-200.png](../artifacts/image-acceptance/final-toolbar-200.png)。
- **原因范围：** image_interaction.rs 的 inspector_panel_rect / 定位策略优先图片下方或上方并约束 viewport，没有正文避让 / 预留布局。边界 panic 和裁剪修复不等于遮挡修复。

### 远程图只显示占位块（未修复 / 能力缺口）

- **缩放 / Markdown：** 200%，DPI 192；`![Remote](https://www.python.org/static/community_logos/python-logo.png)`。
- **步骤：** 打开文档滚到远程条目，等待加载并右键。
- **预期：** 真实远程图片可见且可交互，本地专属操作不出现。
- **实际：** 仅占位块，无远程像素；菜单限制正确。网络对照实际下载同一 URL 成功（45187 字节），不是测试 URL 无法访问。
- **截图：** [remote-context-verified.png](../artifacts/image-acceptance/remote-context-verified.png)；对照 [remote-availability.png](../artifacts/image-acceptance/remote-availability.png)。
- **代码范围：** yu-assets 图片位置解析对远程 / data scheme 返回 UnsupportedScheme；本轮未添加网络加载。这是实际验收未满足项，不以既有能力限制为由标成通过。

### 100% / 125% / 150% 实际 Windows 缩放被阻塞（未验收）

- **当前环境：** 200%，DPI 192，自定义缩放已开启。
- **步骤：** 打开真实设置 → 系统 → 屏幕 → 缩放和布局。
- **预期：** 分别切到三个比例，重开实际窗口重复关键链路。
- **实际：** 设置显示“无法从远程会话更改显示器设置”“自定义缩放比例已设置”；缩放控件禁用，关闭自定义缩放需要注销。
- **截图：** [display-settings-200.png](../artifacts/image-acceptance/display-settings-200.png)。
- 未修改注册表、未注销、未模拟 WM_DPICHANGED。布局单元测试包含 96 / 120 / 144 / 192 DPI，但不能替代实际系统缩放验收。需在可改变显示设置的本机控制台补测。

### 已修复并复测的七处问题

全部在同一真实 200% 会话中复现，重新编译启动后实际操作复测。各例 Markdown 见本文，原始 / 最终截图如下；缺失的原截图明确注明。

| 问题 | 复现、预期与原实际结果 | 原因 / 修复 | 实窗证据 |
| --- | --- | --- | --- |
| 窄窗口 panic | 选中第一张 PNG，缩到 900×1100；应保持可操作，原 min > max abort | 面板最小宽超过 viewport；改有界尺寸和紧凑布局 | 原 [crash-stderr.log](../artifacts/image-acceptance/crash-stderr.log)，新 [13-fixed-narrow-200.png](../artifacts/image-acceptance/13-fixed-narrow-200.png)，窗口存活；遮挡仍另列 |
| 浮动条默认字体极小 | 单击 Local PNG；应清晰可读，原控件字体未随 DPI 放大 | 给字段 / 按钮设置 DPI 字体，DPI 变化更新 | 原 [02-selected-200.png](../artifacts/image-acceptance/02-selected-200.png)，新 [final-toolbar-200.png](../artifacts/image-acceptance/final-toolbar-200.png) |
| 属性窗口布局错误 | 双击 Local PNG；应完整且居中，原重复标题、底部按钮裁切 | 对话框类、DPI 客户区 / 控件 / 字体、owner 居中、Enter / Esc | 原 [04-properties-dialog-200.png](../artifacts/image-acceptance/04-properties-dialog-200.png)，新 [properties-width-draft.png](../artifacts/image-acceptance/properties-width-draft.png)，六组实窗用例通过 |
| 字段内 Esc 不退出选中 | 点图片 → 点 Alt 框 → Esc；应退出，原只转移焦点 | 消息循环补 dismiss | 原失败在原始日志；原截图未单独保留，同名已被复测覆盖。新 [navigation-escape-field.png](../artifacts/image-acceptance/navigation-escape-field.png) |
| 边框越过正文 viewport | 选中第一张图并部分滚出顶端；原描边进入上部 chrome | 描边内嵌并与正文求交，隐藏不可见边而非造假顶边 | 原 [navigation-partial-outside.png](../artifacts/image-acceptance/navigation-partial-outside.png)，新 [navigation-partial-scroll.png](../artifacts/image-acceptance/navigation-partial-scroll.png)，离屏无残留 |
| 转源码后焦点不正确 | Alt 框焦点 → 右键源码 → Ctrl+C；原视觉选中但复制仍走字段 | 转源码后 SetFocus(surface) | 原 [source-first-exact.png](../artifacts/image-acceptance/source-first-exact.png)，新 [source-first-focused-final.png](../artifacts/image-acceptance/source-first-focused-final.png)，相邻 B 精确源码也通过 |
| Explorer 定位失败 | 右键本地 / 中文空格 PNG → 显示；应选文件，原打开桌面 / 未选中 | 规范化路径，用 PIDL + SHOpenFolderAndSelectItems | 原命令行实验见日志，新 [explorer-selection-final.png](../artifacts/image-acceptance/explorer-selection-final.png)，SelectedItems.Path 核对两种完整路径 |

修改仅涉及 platform/windows/yu-shell-windows/src 下的 chrome.rs、image_interaction.rs、native.rs；另新增本报告。

### 验证结果与既有测试失败

- [results-final.json](../artifacts/image-acceptance/results-final.json)：85 个唯一实窗断言成立，覆盖尺寸 / 属性 / 字段 / 导航源码和历史持久化；不包括替换、远程像素或其他系统缩放的通过断言，**不代表全清单通过**。
- [sizes-geometry-final.json](../artifacts/image-acceptance/sizes-geometry-final.json)：十项最终外边界核对成立。
- cargo fmt --all -- --check 通过；cargo clippy -p yu-shell-windows --all-targets -- -D warnings 通过，见 [clippy-final.log](../artifacts/image-acceptance/clippy-final.log)。
- 新增图片裁剪 / 有界布局单测两项通过，见 [image-chrome-tests-final.log](../artifacts/image-acceptance/image-chrome-tests-final.log)；最终编译成功。
- cargo test -p yu-editor image：35 项通过，见 [editor-image-tests-final.log](../artifacts/image-acceptance/editor-image-tests-final.log)。
- shell 全量串行测试：**25 通过、1 失败、1 忽略**。失败为 `native::tests::native_sidebar_navigation_reveals_unicode_search_without_editing_source`，断言背景 canvas 不得遮住 GPU surface。
- 临时精确撤回三个文件补丁，在未修改的 c5878fb6 上单跑该测试，同样失败；随后原样恢复，文件哈希一致。证据：[shell-tests-final.log](../artifacts/image-acceptance/shell-tests-final.log)、[sidebar-baseline-c5878fb6.log](../artifacts/image-acceptance/sidebar-baseline-c5878fb6.log)。不修无关 sidebar，也不声称全量全绿。

## 证据和剩余工作

证据目录 artifacts/image-acceptance 包含实际 PNG、HWND JSON、events.jsonl 操作时间线、初始 / 最终 Markdown、图片、脚本、结果和崩溃事件。desktop.ps1 为真实桌面助手；matrix.ps1 有 sizes / properties / fields / navigation 套件。重跑会真正修改文档、剪贴板和窗口，应先备份需保留的文档并确认前台。

早期菜单步数、IME 未上屏、连续点击成为三击等脚本校准日志保留，最终以去重 results-final.json、最终截图及 geometry 文件为准。

尚需：定位并修复选择器崩溃后重测两条替换链路；改善浮动条避让；实现 / 确认远程真实图片加载；在实际 100% / 125% / 150% 系统显示环境补测。当前不出具完整验收通过结论。
