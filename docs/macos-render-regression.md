# macOS 自动窗口回归与性能采样

当前状态：下文首次发现的长文档末行问题已修复，最终窗口回归通过。
历史失败与 Instruments 数据仍保留，修复后的结果见文末；连续触控板性能尚未验收。

## 运行入口

在正常 macOS 图形会话中运行，使用 Xcode 与本仓库的 debug app：

```sh
./platform/macos/yu-shell-macos/build-app.sh
./platform/macos/yu-shell-macos/run-render-regression.py .notes/render-regression-new
```

输出目录必须不存在，避免覆盖历史证据。`--case long` 或 `--case resources`
只重跑对应场景；`--prepare-only` 只生成测试文档与图片，不启动 app。
每次记录二进制/fixture SHA-256、Git 状态、硬件、macOS、Xcode、窗口尺寸、
scale、屏幕最大刷新率、原始日志和各阶段 p50/p95/p99。

### 自动化覆盖

- 324,245 字节长文档，含 350 个章节、中英/RTL/组合字符、表格和引用。
- 首帧在顶部，12 次分段滚动、4 次窗口缩放，内容高度与滚动范围一致。
- 导航到末行：同时检查 caret 查询与已提交 Metal 帧中的 caret，不能只看
  查询返回 `needsScroll=false`。
- detach/rebind；实际关闭 NSWindow 后，用新 session/window 重开并验证顶部首帧。
- 图片延迟 14 秒、Math 延迟 16 秒，同时保留 Mermaid `Unsupported`。
- pending 时 coverage 内滚动保持 frame serial，使用 retained presentation。
- 停止操作后只读取 coordinator 已提交的 snapshot，不提交帧、不查询资源，
  验证完成通知能主动更新窗口；图片高度只变一次，完成后不再空转发布。
- 正常 pending 不得触发 `resource_retry` 定时重试。

延迟只在 Rust `debug_assertions` 构建启用：`YU_TEST_IMAGE_DELAY_MS` 与
`YU_TEST_MATH_DELAY_MS` 接受 1–30,000ms，release 构建不包含注入逻辑。
测试不模拟 live-scroll 起止事件，所以不会把脚本驱动的帧间隔计入真实触控板验收。
资源测试验证 Math pending/通知状态，不证明 Math 内容在画面中的最终视觉质量。

## Instruments

使用生成的 fixture 录制相同的自动场景：

```sh
./platform/macos/yu-shell-macos/record-instruments.sh \
  .notes/render-regression-new/fixtures/long.md \
  .notes/instruments-long-new 60s --render-regression

./platform/macos/yu-shell-macos/record-instruments.sh \
  .notes/render-regression-new/fixtures/resources.md \
  .notes/instruments-resources-new 60s --resource-regression
```

自动场景失败时保留 trace、日志和统计结果，并以非零状态退出。导出 CPU 调用栈：

```sh
xcrun xctrace export --input .notes/instruments-long-new/render.trace \
  --xpath '/trace-toc/run[@number="1"]/data/table[@schema="time-profile"]' \
  --output .notes/instruments-long-new/time-profile.xml
```

不传自检模式时是实际交互采样，需要用户在打开的窗口中连续触控板滚动。
应先约好采样时间，再开始录制；尚未完成的手势和没有样本不能被判为达标。

## 2026-09-11 实测

硬件 Apple M1 Max（Mac13,1），macOS 26.5 / 25F71，Xcode 26.6 / 17F113。
debug 构建，2x，屏幕报告最大 60Hz。首次窗口 900×692pt；重开窗口 900×652pt。

| 场景 | 结果 | 证据 |
| --- | --- | --- |
| 延迟图片、Math、Mermaid 状态 | 通过 | 2 次完成通知，0 次资源定时重试；pending 滚动保留 publication；闲置后完成更新，高度改变 1 次 |
| 长文档顶部、滚动范围、12 次定位、4 次 resize | 通过对应断言 | 每次均提交非空当前 Revision 帧，extent 与 publication 相符 |
| 长文档末行导航 | **失败** | 查询认为 caret 已可见，但实际帧 caret 数量为 0 |
| detach/rebind 与实际关闭/重开 | 通过对应断言 | 新 publication serial；重开回到顶部且提交首帧 |
| 连续触控板 p95 ≤ 16.7ms | **未验收** | 本轮仅脚本输入，无 live-scroll 手势样本 |

原始证据：

- `.notes/macos-render-regression-20260911-a/`：资源通过、长文档首次失败。
- `.notes/macos-render-regression-20260911-b/`：按正确正文宽度重测末行，仍失败；
  独立完成关闭/重开检查。`long-summary.json` 保持 `passed=false`。
- `.notes/macos-instruments-long-20260911-a/`：真实 Metal System Trace、CPU
  time-profile 导出和统计，目标应用因末行断言退出；xctrace 返回 54，不能当作通过。

末行复现值：caret 查询 Y=28,001.8125pt，当前 scrollY=27,415.5pt，
viewport=624pt，`needsScroll=false`；已提交帧 `caretDecorationCount=0`，
内容高度=31,393.6875pt。代码中 caret 查询使用 canonical editor 的增量高度状态，
而 `publish_owned` 重建独立 EditorDocument，输入只传 ViewportConfig，没有传递
测量后的高度状态。这是定位与渲染不一致的明确修复方向，仍需实现与回归验证。

非 Instruments 自动长文档采样：后台 preparation p50=615.55ms、p95=666.47ms；
Swift submit attempt p99=561.99ms；Metal encode/submit p95=1.23ms。
这些是离散脚本步骤的阶段耗时，不是连续呈现帧时间。

Instruments CPU 采样中，主线程共约 16.271s 样本权重，
`DocumentTextView.refreshTableResizeAccessibility` 的包含子调用权重约 4.887s；
下游 `yu_storage_session_table_resize_accessibility_dividers` 约 4.693s，
并调用 `visible_blocks_with_shaper`。这些数值相互包含，不能相加；它们说明
主线程仍有辅助功能几何查询触发的排版工作，不能仅凭 Metal 编码较快宣称流畅。

下一步应先统一长文档定位与 publication 的布局状态，再分析并减少这条主线程
重复布局路径。真实 IME、VoiceOver 操作、跨显示器、残影与触控板体验仍需人工配合。

新增入口后的基线检查：app 构建成功，原有 Dark Aqua 真实窗口 self-check 通过；
`yu-render-macos` 10 项通过、2 项原有 ignored，metrics 统计测试 3 项通过。
这里的通过不覆盖上表明确失败的长文档末行用例。

## 布局一致性修复后的复测

最终证据保存在 `.notes/macos-layout-consistency-20260911-accepted/`，硬件、
macOS 和 Xcode 与上文相同，2x / 60Hz。长文档和资源场景均以退出码 0 结束。

此次修复包含：

1. `EditorRenderSnapshot` 携带已测量的 viewport 块高度，后台不再从默认估算
   重新开始。publication 返回自己的高度状态，主线程在接收验证通过后采用；
   不共享可变 EditorDocument，也不接收 worker 中带字体 face ID 的 layout cache。
2. 文档身份、Revision、选区、composition 和 viewport 配置共同保护高度状态
   的接收，host 原有 request/surface/binding/resource generation 校验仍保留。
   编辑和 reset_source 会更新身份，避免新文档重用初始 Revision 时接收旧结果。
3. 单独改变 overscan 不再清空已测高度。捕获会复制标量高度索引，开销随块数
   增长；这并非零成本，也不等于完成所有长文档性能优化。
4. coordinator 只缓存正文 inset，从当前 surface 推导排版宽度；surface 同时
   跟随 clip view 的最终 frame/bounds 更新，避免 NSScrollView 在 controller
   布局回调后才完成 resize 时留下旧尺寸。
5. 后台实测高度改变导航目标后，继续完成同一次 caret reveal。后续滚动、
   选区/Revision 变化和 detach 取消旧意图；普通布局回调不再新建导航请求。
   提交过程中若 AppKit 几何改变，等待当前尺寸的新帧，不返回旧尺寸结果。

末行判据同时检查已提交帧中的 caret 和完整 caret 矩形是否位于 viewport，
允许 0.5pt 取整误差。原先把 `needsScroll=false` 当作必须条件，会把 AppKit
对 0.24pt 的像素取整误判为失败；旧缺陷的 `caretDecorationCount=0` 仍无法通过。

最终窗口结果：

- 长文档：324,245 字节，12 次定位、4 次 resize，宽度断言、末行可见、后续
  滚动取消导航、detach/rebind、关闭后重开并回到顶部均通过。
- 资源：图片 14 秒、Math 16 秒延迟，pending retained reuse、idle completion、
  图片高度改变 1 次，以及无正常 pending 重试定时器均通过。
- 原有 Dark Aqua 窗口检查通过，包含搜索、caret、多光标、大纲与 resize。
- 最终二进制的 14 项 shell self-check 全部通过，包含表格、剪贴板和输入协议。
- Rust 回归：`yu-editor` 102/102、`yu-storage-ffi` 48/48、`yu-workspace` 46/46。
  新增测试覆盖高度往返传递、过期/跨文档/不同视觉状态拒绝、overscan 保留测量。

实现分别提交为 `1a06fa6`（布局测量传递）与 `5c9875f`（窗口几何与导航恢复）。

此次完成的是定位与渲染正确性修复。主线程辅助功能查询的排版成本仍需后续分析，
不能据此将真实触控板 p95、IME、VoiceOver 或跨显示器人工验收标记为完成。

## 表格辅助功能几何复用（2026-09-11）

表格列分隔符描述现在从已提交 frame 的纯几何快照生成。快照在 scene assembly
使用最终 table layout（含列宽 override）时捕获，不携带 CoreText 或 glyph cache。
附着窗口的查询校验 build key、surface generation、已提交 serial 与 viewport
coverage；未提交、过期或 composition 状态返回空，等待新帧触发刷新，不同步排版。
无窗口协议调用保留原路径；Swift 数量查询为零时不再发起第二次填充调用。

最终自动证据：`.notes/macos-ax-frame-20260911-accepted/`。机器为 Apple M1 Max
（Mac13,1），macOS 26.5 / 25F71，Xcode 26.6 / 17F113，debug build，2x / 60Hz，
重开窗口内容尺寸 900×620pt；长文档仍包含 12 次定位和 4 次 resize。

- 长文档和延迟图片/Math 均通过，原有末行、导航取消、关闭重开检查保持通过。
- 新增真实窗口检查：表格 AX increment 后新帧列分隔符移动一个 adjustStep，
  Markdown 源码与表格 source range 不变；新列宽尚未提交与 detach 后均无旧描述符。
- 长文档查询 95 次，匹配已提交几何 42 次；资源查询 16 次，匹配 11 次。
  两者 `ax_layout_fallback` 均为 0。runner 已将无回退、有帧命中纳入通过条件。
- `yu-workspace` 46/46（含最终绘制与表格元数据一致性断言）、`yu-storage-ffi`
  48/48、14 项 shell self-check 与 Dark Aqua 真实窗口 self-check 均通过。

Instruments 证据：`.notes/macos-ax-instruments-20260911-a/`，含 trace、导出的
`time-profile.xml` 与 `cpu-summary.json`。xctrace 退出码 0，长文档关闭重开完成。
该 trace 使用相同生产实现、添加最终两个失效断言之前的二进制；两组 binary SHA
分别保留在各自 environment 文件中。

此次主线程样本权重 15.668s，`refreshTableResizeAccessibility` 包含子调用权重
146ms；同时包含该函数与 `visible_blocks_with_shaper` 的样本为 0。日志记录
90 次帧查询、41 次几何命中、0 次同步回退。这支持指定 AX 排版路径已移除，
不代表查询绝对零耗时。旧 trace 的 4.887s 包含布局一致性修复前的行为，且本次
新增列宽操作，因此不把两次采样当作受控的整体提速倍率。

最终无 Instruments 的脚本样本中，Swift submit attempt p95 6.697ms / p99
26.891ms，Metal encode/submit p95 1.126ms。后台 preparation p95 722.914ms，
bounds-to-request p95 287.840ms，仍有明显长尾；这些分布覆盖脚本导航和 resize，
不能代替连续触控板帧间隔。观测到的 GPU in-flight 最大值为 1。

下一项有采样证据的主线程路径是 `mouseMoved → tableResizeHover →
yu_storage_session_table_resize_at_point`（包含子调用约 1.753s），仍触发可见块
排版。另有启动/选区同步成本，需分别定位。此次不扩展修改这些路径。
真实手势 p95 ≤ 16.7ms、残影、VoiceOver 交互、IME 和跨显示器验收仍未完成。

## 表格悬停复用已提交几何（2026-09-11）

`tableResizeHover` 现在调用独立的只读 Rust hover 查询，从已提交 frame 的
`ViewportTableGeometry` 判断列分隔线。与 AX 枚举共用 publication 有效性校验，
但不构造 AX 描述符或转换 source UTF-16，也不创建 CoreText shaper。原有完整
PROBE / BEGIN / UPDATE / FINISH 路径保留，用于实际拖动操作；无窗口协议检查
保留同步 hover fallback。窗口初始未附着或 detach 后返回普通光标。

证据目录 `.notes/macos-hover-frame-20260911-a/`：Apple M1 Max / Mac13,1，
macOS 26.5 / 25F71、Xcode 26.6 / 17F113，debug，2x / 60Hz；窗口尺寸与 fixture
同上，binary SHA 和完整运行环境已记录。

- Rust 46 项 workspace 与 48 项 FFI 检查通过。几何检查与原 TableLayout
  命中测试逐点比较，覆盖容差边界、上下边界及非零文档偏移；FFI 验证命中、
  未命中、过期 Revision 与非有限坐标，并检查失败输出清零。
- 长文档及延迟资源真实窗口检查通过。新增连续 100 次悬停不启动手势或改变
  publication、表格范围外不命中、未提交新列宽不命中、列宽提交后跟随新位置、
  detach 后不命中的断言。14 项协议 self-check 和 Dark Aqua 窗口检查通过。
- 长文档共 104 次 `hover_frame_query`，`hover_layout_fallback=0`；两组场景
  `ax_layout_fallback` 也均为 0。runner 要求长文档至少 100 次帧悬停查询，且
  两组场景均无 hover 同步回退。
- 无 Instruments 的完整 Swift `hover_query` 共 105 个样本（含 detach 的提前
  返回），p50 1.668ms / p95 1.780ms / p99 1.789ms。计时包含 bridge 状态读取，
  不包含外层 metric 输出；开启 timing，故不作为正式交互性能验收。

连续触控板帧间隔仍无样本，后台 preparation 的脚本 p95 633.858ms 也仍有长尾。
本次只消除悬停查询中的同步排版，不据此宣称整体滚动达到 16.7ms。

Instruments 第一份 `.notes/macos-hover-instruments-20260911-a/` 的应用自检完成，
但 xctrace 保存退出码为 1，导出报 `Document Missing Template Error`；该 trace
不作为采样证据。重录的 `.notes/macos-hover-instruments-20260911-b/` 以退出码 0
保存，CPU XML 成功导出，二进制与上述自动回归一致。

有效 trace 主线程累计样本权重 15.711s，悬停包含子调用权重 250ms，AX 刷新
147ms；同时包含各自入口与 `visible_blocks_with_shaper` 的样本均为 0。通用
`table_resize_at_point` 仍有 68ms 样本，用于实际列宽操作，未宣称所有主线程
表格排版都已移除。脚本比旧 trace 多了 100 次主动 hover，不作为受控提速倍率。

悬停剩余样本中，247ms 包含 `yu_storage_session_state`，242ms 包含
`current_fingerprint`。代码确认 Swift 获取 `bridge.state.revision` 会连带执行
`disk_state → current_fingerprint → fs::read`。下一项应将高频 Revision 查询
与磁盘外部修改检测分离，保留保存/重新加载时的冲突校验；本次尚未修改这部分。
上述 inclusive 样本相互包含，不能相加，也不是单次查询时长。

## 高频 Revision 查询不再检查磁盘（2026-09-11）

新增 `yu_storage_session_revision`，只读取 canonical session 的内存 Revision。
Swift 的 `bridge.revision` 每次通过此接口读取，不复用可能过期的完整状态缓存。
渲染、hover、AX 几何、选区、输入、composition 和面板查询中单独使用 Revision
的位置均已迁移。`bridge.state` 与保存、重新加载、关闭时的磁盘冲突逻辑不变，
状态栏/菜单需要完整状态时仍会检查磁盘；本次不是移除外部修改检测。

自动证据位于 `.notes/macos-revision-query-20260911-a/`。机器与前述相同：
Apple M1 Max / Mac13,1，macOS 26.5 / 25F71，Xcode 26.6 / 17F113，debug，
2x / 60Hz，窗口与长文档/延迟资源 fixture 同前，环境和 binary SHA 已保存。

- 49 项 FFI 测试通过。新增测试验证编辑后立即读到新 Revision；将磁盘文件
  替换成目录导致完整状态检查失败时仍能读取最新内存 Revision；恢复为外部
  修改内容后，完整状态报告 Changed，保存拒绝覆盖，磁盘原内容保持不变。
- 14 项协议 self-check、长文档、延迟资源和 Dark Aqua 窗口检查全部通过。
- 长文档 133 个 hover 样本：p50 0.003750ms / p95 0.050459ms / p99 0.057ms。
  上一版同 fixture 的 p95 为 1.780ms，但运行中的实际鼠标事件数不同，不作为
  受控提速倍率。hover / AX 同步排版 fallback 均为 0。
- Swift submit attempt p95 1.550625ms / p99 24.790542ms；Metal encode/submit
  p95 0.592583ms。后台 preparation p95 622.687708ms，仍有长尾。

测量流程注意：`run-dark-self-check.sh` 当前使用 `pkill -f`，会匹配 xctrace
命令行中包含的应用路径。必须等 Instruments 保存和导出完全结束后，才能运行
深色自检；仅等目标应用退出还不够。本轮第一份 `macos-revision-instruments-20260911-a`
在保存期间因此被终止（xctrace_exit=1），不能作为 CPU 证据。之前 hover 第一份
保存失败也发生在相同的并发脚本顺序中。

串行重录 `.notes/macos-revision-instruments-20260911-b/` 成功，xctrace_exit=0，
CPU XML 可导出，`cpu-summary.json` 保留线程权重、完整主线程 inclusive 排名和
路径交叉计数。binary SHA 与自动回归一致。主线程样本权重为 14.663s；hover、
submitNow、AX 刷新各自调用链中，`yu_storage_session_state`、`current_fingerprint`
与 `visible_blocks_with_shaper` 的交叉样本均为 0。hover 本身采样到 1ms，
submitNow 包含子调用 535ms；不应将采样未命中解释为绝对零耗时。

完整状态函数仍有 64ms、磁盘指纹读取仍有 66ms 主线程样本，属于其他调用链。
说明本次移除了高频路径中的连带检查，而非删除磁盘检查。初始化/选区同步链
仍有约 3.9s inclusive 样本，两个 frame worker 分别约 6.459s / 0.386s，
需要后续单独分析，不能根据这些重叠权重直接推导连续滚动帧率。
真实触控板 p95 ≤ 16.7ms、IME、VoiceOver 与跨显示器验收仍未完成。
