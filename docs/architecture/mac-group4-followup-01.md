# 第四组收尾第一轮：真实选择、列宽、综合文档与资源压力

更新：2026-09-26。基线 `25f1e936ff066c1d4c685ddc7263178cfa22fb4e`。本轮通过 MCP 在 `yu-workspace` 操作、打包和检查，修复分批推送 `main`，不等全部验收完成再提交。未修改 CI 或开发导出。

**第四组仍未全部结项。** 本轮缩小已知交互和综合文档缺口；真实午夜／多日睡眠／系统时区事件、多文档长时压力与全部输入排列仍不计通过。

## 版本与交付

| 项目 | 实际值 |
| --- | --- |
| 第一项修复 | `e4e2cf35efd677a665a589ca8dd4a451838ad991`，列宽分隔线 AX 屏幕坐标；完成后即推送 |
| 第二项修复／最终受测产品源码 | `c747ea130a096f068426da77c38bac47862fff94`，跨条目选择期间稳定 HTML 列表投影；完成后即推送 |
| 最终应用主程序 SHA256 | `3e5548cf175569231eebd61973670ee213b80f08590c34d329a1d5c7ac8e6ace` |
| 随包 helper SHA256 | `f74c4f402bf005eee852db83f272de35c512d43bffb5e868c84da32ca769f68e`，未改变 |
| 环境 | Release、arm64、macOS 27.0、Xcode 27.0；不是 macOS 26 真机结果 |
| 原始证据目录 | `artifacts/group4-followup-20260926-01/` |

产品在提交前工作副本完成构建，随后相同产品源码提交到上述 SHA；仅测试诊断文字与文档的后续修改不改变已打包产品代码。每组结果记录构建、隔离重签后的应用、helper、执行脚本和 follow-up 模块哈希。重新打包必须记录自己的产物身份，不照抄本页 SHA256。

收尾时发现一个非本轮创建的未跟踪目录，未查看其内容、未提交或删除；它不属于本轮交付。不要将“本轮代码已全部提交”写成整个工作区绝对干净。

旧 `group4-freeze-20260926-rc3` 标签仍指向 `6afb7a3d…`，没有移动。当前修复后的应用不是旧 RC3 二进制；RC3 的1541项与16组实窗结果保留为历史，不计成本轮重跑。

## 1. 分隔线坐标修复

外部 `AXSplitters` 可读取分隔线，但原 `tableResizeAccessibilityFrame()` 使用 surface 坐标，漏掉实际阅读区水平边距和顶部内边距。1200×800pt 场景中，分隔线报告横坐标相差100pt，按该位置实际拖动未命中分隔线。

修复使用 `DocumentTextView.contentOrigin`，将 Rust 内容坐标转换到文档视图，再交 AppKit 统一处理滚动和屏幕转换，与输入点的逆变换一致。解绑、旧 revision 和组字状态仍拒绝提供失效几何；未新建另一套列宽计算。

浅深色分别通过真实合并列宽拖动55pt、Escape取消35pt预览、源码不变和完全退出重开。重开后先恢复同样1200×800pt视口再比较，避免把正常宽度自适应误判成持久化失败。最终相对分隔线位置均回到550.430374pt。这个成功不能直接代表所有窄窗口、缩放、屏幕切换和 VoiceOver 操作均完成。

## 2. 跨层级列表拖选修复

新增真实反向拖选时发现：子项至父项的选区在进入父项后，父项立即展开原始 HTML 标签，文字位置随之改变；后续鼠标事件落入 `<li id=…>`，本来合法的缩进被底层正确地安全拒绝。

原单光标静态命中回归可以通过，但加入已跨条目的活动选区后，原生 CoreText／FFI 回归稳定失败：父项正文预期UTF-16位置50，实际命中34。修复限定为**非空且跨越 HTML 列表分区的选区保持原生投影**。单项内部的光标／文字编辑、显式源码模式和拒绝非法范围的规则保留，不全面取消源码显示。

修复后通过独立列表／表格单元格 × 反向鼠标拖选／父子Option点击双光标 × 浅深色。实际 Cmd+] 只移动父项及其子树一次；未编辑邻居、全部可观察AX选区、一次撤销、重做、BOM／CRLF保存及完整退出重开均核对。AX范围不能证明其未暴露的affinity等内部字段；这些仍由核心回归提供证据。

源记录保留 `list-gestures-light*`、`list-gestures-active/` 和 `list-drag-core-before.log`，不删除失败截图或修改断言求通过。选择点使用可见字形的范围，不把包含隐藏边界的多行矩形中点当成文字位置。

## 3. 最终构建上已执行的交互

连同下方有限压力，最终同产品构建共7组、55条脚本检查记录通过；这不是55个独立Rust测试函数，未将早期构建或重复跑法累加。每组均完成保存、退出、新PID重开、继续编辑和再次退出；结束后检查未发现本轮隔离应用／helper残留。机器可读结果见 [summary.json](evidence/group4-followup-20260926-01/summary.json)。

| 目录／入口 | 已执行范围 |
| --- | --- |
| `final-tables-light/dark`；`--table-interactions --table-resize --reopen` | 每主题四种整表组合：正／反向 × 成功／拒绝。实际复制donor，再Option+Shift拖满目标表格；校验两个owner范围、thead／tbody和空tbody。拒绝后完整回放原A/B撤销重做分支；另验合并列宽拖动、取消、重开 |
| `list-fixed-light/dark`；`--list-gestures --reopen` | 每主题四种列表选择场景：独立／单元格 × 反向拖选／双光标。真实按键缩进、一次子树移动及选区／源码历史 |
| `final-smoke-light/dark`；`--smoke-document --reopen` | 整份固定 `group4-smoke.md` 加相对图片。真实目录点击跳转、折叠指针开关、搜索隐藏正文、正文编辑、历史、精确保存和重开 |

目录点击使用固定语料的15个短标题及生成目录的实际AX范围，且验证最终落到指定源码；没有使用OCR定位或把点击标题本体冒充目录跳转。该测试只验证其中一个目录链接，并非所有目录行均逐一点击。

综合文档按15个标题位置分别捕获窗口。实际复核最终构建8个窗口截图组成的两页联系表，覆盖整表粘贴、列宽、列表选区和层级、目录、折叠正文及混排内容。截图生成不自动等于全部长页视觉验收；实际复核范围在机器记录中列明。有限HTML当前不支持 `blockquote`：综合样本中的该标签保持原始源码回退，不将它计作原生引用布局成功，也未在本轮扩展标签白名单。原始黑色logo在深色背景对比度低仍是样本边界，没有重新着色。

## 4. 检查结果

| 检查 | 实际结果 |
| --- | --- |
| `cargo test --locked -p yu-markdown -p yu-editor -p yu-storage-ffi` | 872通过、0失败、0忽略；包含新增原生列表命中回归及既有表格／公式／脚注检查 |
| 上述三个crate全部target的Clippy，`-D warnings` | 通过；初次新增测试的unwrap警告已改为带说明的expect，未放宽lint |
| 全仓 `cargo fmt --all --check` | 通过 |
| 最终Release构建与随包产物审计 | 通过，见 `build-list-fix.log` |
| 最终应用 `run-self-checks.sh` | 19项原生检查全部通过，另包含Swift日历7组、调度器／打字机几何／系统拼写／图片文件检查；`native-final.log` |
| `tools/test_group4_followup.py` | 8项通过，只证明测试工具参数边界／UTF-16换算等，不计为产品实窗测试 |

## 5. 有限资源压力

最终构建的300秒任务已退出并返回0，实际完成73轮，含最终恢复步骤计时303.057秒。应用物理足迹样本范围为85,034,064至122,471,528字节，后段仍有增长；尚未区分编辑历史、缓存保留与其他增长来源，不宣称内存稳定或无泄漏。这个结果来自 `final-stress-300s`，不是复用早期构建恰好同样73轮的记录。

循环交替有效公式／图表、无效图表、普通文档，并检查源码、撤销重做和进程数量；结束恢复有效文档并完全退出重开。完成请求的helper可以合法等待既有空闲计时器，不能以立即退出作为错误判据。普通文档不得额外启动进程，渲染时最多一个helper。

该循环是有限、单文档行为压力，不证明所有错误图在下一次修改前均已完成像素呈现，也不证明长期无泄漏、多文档峰值或后台睡眠行为。内存足迹范围只作为观测值，不虚构通过阈值。

## 6. 剩余范围与复现

真实午夜、多日睡眠唤醒、系统时区／时钟事件依然未执行；没有修改用户系统时钟。长时多文档压力、76份表格及96份列表全部排列、更多缩放／混合选区、长时序图全图和所有相邻写作流程也不因本轮通过而自动完成。

可用新输出目录复现（从仓库根目录运行，先构建并核对 `.build/build-manifest.json`）：

```sh
python3 platform/macos/yu-shell-macos/run-embedded-checks.py --table-interactions --table-resize --reopen artifacts/group4-next/tables-light
python3 platform/macos/yu-shell-macos/run-embedded-checks.py --list-gestures --dark --reopen artifacts/group4-next/list-dark
python3 platform/macos/yu-shell-macos/run-embedded-checks.py --smoke-document --reopen artifacts/group4-next/smoke-light
python3 platform/macos/yu-shell-macos/run-embedded-checks.py --stress-seconds 300 --reopen artifacts/group4-next/stress
python3 -m unittest discover -s tools -p 'test_group4_followup.py' -v
```

除两个表格模式可以组合外，各新专项互斥，只搭配主题和重开选项；冲突和非法时长在创建输出目录或启动应用之前拒绝。旧专项保持原有验证，不删除既有边界测试。
