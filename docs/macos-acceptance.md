# macOS 基础体验验收记录

最新补充（2026-09-11）：表格无障碍枚举已改为读取已提交帧几何，真实窗口
长文档/延迟资源检查未发生同步排版回退；列宽操作及失效描述符检查通过。
Instruments 未采样到 AX 刷新调用链中的可见块排版。完整证据与剩余热点见
[表格辅助功能几何复用](macos-render-regression.md#表格辅助功能几何复用2026-09-11)。
这不等于真实 VoiceOver 或连续触控板性能验收完成。

本轮在真实 macOS 窗口中完成了以下检查：

- [x] 原生 toolbar：保存、重新加载、大纲、搜索图标和辅助功能描述。
- [x] Yu 侧栏标题、logo、sidebar material 和大纲层级行。
- [x] 大纲点击导航到远端标题，并保持正文定位。
- [x] 搜索面板显示匹配数量、结果列表和正文高亮。
- [x] 侧栏隐藏/恢复时正文布局和滚动位置保持有效。
- [x] 滚动操作后的最终画面与滚动条位置对应；静态截图不能证明连续帧率、
  中途无残影或无 beachball，相关性能验收仍未完成。
- [x] Dark Aqua 真实窗口 self-check：窗口外观断言、Metal frame 提交、caret/
  selection/search/multi-cursor/outline/extent 检查通过。
- [x] 清洁文档 toolbar 状态：真实窗口 self-check 断言 Save/Reload 均为 disabled。

## Timing sample

优化前，`YU_RENDER_TIMING=1 ./run-dark-self-check.sh` 在当前开发机真实窗口采样到：

- retained presentation：约 `0.25–0.33ms` total；
- full frame build：约 `1.28–1.42s` total。
- 旧 phase breakdown 的 layout/publish 计时在函数末尾结束，区间重叠，不能据此
  断言两者各耗时约 1.25 秒；已改为各阶段结束时取样，等待重测。

代码检查发现 RenderPlan 对每个 glyph 重复哈希整张 atlas page，现改为每次
build 每页一次，保持逐 glyph entry 校验与跨 build page mutation 检测。

优化后，同一 `outline.md` fixture、debug app、Dark Aqua real-window self-check
以退出码 0 完成，采样结果：

- 冷启动首次 submit：`50.545ms`；
- 热态完整 submit：`9.978–12.406ms`；
- retained submit：`0.249–0.754ms`；
- viewport probe（日志 layout 字段）：`0.650–2.966ms`；
- publication：热态 `8.759–9.474ms`，首次 `22.564ms`。

以较大的仓库 `README.md`（约 8.5KB）运行 timing 时，首次 full submit 为
`107.776ms`（build `101.861ms`），随后 warm submit 约 `12.3–12.7ms`，retained submit 约
`0.39–0.85ms`。README 不包含 frame scheduling self-check 所需的多光标场景，
因此该命令在多光标断言处停止；此结果只用于性能采样。

29 个 yu-render 测试通过，包括 256 glyph 共用一页时只哈希一次的计数断言、
重复 build 不重复上传，以及 page mutation 仍触发上传。AppKit toolbar
自动验证现通过 NSToolbarItemValidation 读取 Rust 状态，真实窗口断言通过。

上述小样本不是长文档连续滚动 p95，不能据此宣称性能验收完成。当时后台构帧与
异步发布尚未完成；后续实现进展见文末记录，长延迟资源场景仍需专门验收。

GPU 提交现增加零等待的信号量限流，暂时无法提交通过独立的
`YU_STORAGE_RENDER_BUSY` 返回。Swift 保留已有画面和资源刷新意图，按显示帧间隔
重试最新状态；成功提交或 detach 时取消重试。

drawable 获取现由后台任务执行，主线程只取已准备的结果，每个 layer 最多一个
获取任务。resize/detach 推进 generation 并丢弃旧结果；尺寸不匹配时重试。
`nextDrawable` 本身仍可等待，但不再位于主线程提交路径。原生阻塞注入测试
验证获取尚未返回时 1000 次请求及失效/detach 不等待获取完成。
受限环境首次测试用时 65.78s（包括原生初始化），不能作为帧耗时证据。
这条路径尚需真实窗口强制 backpressure、resize 和关闭窗口场景验收。

最新 Rust 静态库重新链接后，Dark Aqua 真实窗口 self-check 退出码为 0：
首帧、retained coverage 滚动、搜索、caret、多选区、大纲导航和代码高亮通过。
自检已改为 async，在 drawable 暂未就绪时让出主线程，3 秒超时仍判失败。
新增窗口扩大 80x40pt 再恢复的检查，两次均验证 surface generation 推进且
新帧成功提交。该检查没有测量连续滚动 p95，也没有逐帧检查残影。

资源刷新修复：host 保存 `resource_refresh_pending`，retained snapshot 不再
把它重置为 0。pending 期间禁止通过 current-frame/coverage 判断跳过构帧，
确保轮询能收取 worker 结果。此前 Swift force 仅绕过 Swift 的比较，Rust
仍可能复用占位帧并停止刷新。

图片仍在解码且结果通道为空时，coverage 内允许复用帧并保留 pending 标志；
结果到达后重建并收取结果。worker 的 readiness probe 保留结果及返回顺序。
这减少等待图片期间的重复 layout，尚未消除定时轮询。

Math 渲染已移入 `yu-math` worker，状态查询只派发请求和收取已完成结果；
缓存仍在 owner 线程按当前 Revision 完成发布，过期结果被丢弃。销毁 owner
不 join 渲染线程。结果入队后通过合并的主线程通知唤醒仍 pending 的窗口。

图片 worker 的销毁也不再 join：设置取消标记并关闭请求通道，正在执行的
ImageIO 调用返回后退出，未开始的任务不再解码。阻塞解码测试验证 owner
能先完成销毁，随后 worker 退出且不执行排队任务。实际慢文件窗口关闭仍需
交互式验证。

图片/Math worker 现于结果入队后发出合并的主线程通知，Swift 只唤醒仍 pending
的 attached 窗口。通知不携带文档指针，observer 在 coordinator 销毁时移除。
完成代数参与帧复用检查，构帧期间到达的结果不会被标成已消费。当前通知为
进程级，可能让其他 pending 窗口多检查一次；退避轮询仍保留用于失败重试。
完整 app 已重新链接，Dark Aqua 窗口及 resize 回归通过；慢资源超过轮询预算
后的通知刷新仍需专门验证，不能用 outline fixture 的通过替代。

正常 worker pending 已不再安排定时轮询。Rust 新增 `resource_retry_pending`
区分失败重试与完成通知等待，Swift 仅对 retry 安排有界退避。通知超过旧轮询
预算后仍能唤醒 pending 窗口；该长延迟场景的端到端验证仍待完成。

仍需在目标硬件上用 Instruments 完成的量化验收：

- [ ] 连续滚动帧时间 p95 ≤ 16.7ms。
- [ ] Core Animation / Metal 无 drawable 等待或 command-buffer 堆积。
- [ ] Retina、窗口 resize、图片加载、IME 和表格 resize 回归。

Headless self-check 的 clipboard/document-workflow 仍依赖系统 NSPasteboard；在
受限自动化环境中会因 pasteboard 权限失败，不能替代交互式 macOS 验收。

## 2026-09-11 后台准备边界记录

当前开发机为 Apple Silicon macOS 26.5，Xcode 26.6。Rust `yu-storage-ffi`
43 个单元测试、workspace frame-builder 4 个测试均通过，debug app 已重新链接。
surface 提交现在先捕获 owned document snapshot，由 worker 生成 publication 与
匹配的 CPU glyph atlas；主线程校验 `FrameBuildRequest` 的 key/generation 后再
同步 atlas、获取 drawable、编码并提交 Metal。worker 创建失败时保留同步 owned
publication 作为恢复路径。这一版 detach 会释放 worker、下次绑定重新创建；
文末记录的后续修订已改成保留单 worker 并使旧请求失效。

后续定位确认 `Native surface inactive`（Rust status 21）是后台接入的代码回归，
不是目标机器缺少 Metal：host 已接收 worker publication，但 snapshot 仍读取
主线程 builder 中不存在的 publication。现统一从 host 已接收的 publication
读取元数据，worker 重建从同一发布序号继续，detach 重置旧 surface generation。
新增实际走 worker 的回归测试，修复前复现 `Err(21)`，修复后覆盖后台首帧、
同 Revision 请求替换、resize 后 detach/rebind，并通过。

修复后的 `run-dark-self-check.sh` 在正常图形会话中退出码 0：首帧、retained
滚动、搜索 `1→0`、caret `1→2`、多选区、大纲导航 `156→1190`、代码高亮及
两次窗口 resize 提交通过。此结果仍不是连续触控板滚动 p95 或完整人工验收。

### Instruments 入口实测（2026-09-11）

`record-instruments.sh ... 30s --self-check` 已在 Apple M1 Max（Mac13,1）、
macOS 26.5 / Xcode 26.6 上成功生成 Metal System Trace。原始数据位于本地
`.notes/macos-instruments-20260911-smoke/`（不提交 trace 二进制）。debug app 自检完成，
覆盖首帧、滚动定位、搜索、多光标、resize 与 detach/rebind。

- 实际 drawable 呈现 14 次，GPU submit/complete 各 14 次；按 surface 观测的最大
  in-flight 为 1，GPU busy 为 0。
- drawable 尚未就绪 16 次；Swift render busy 51 次，包含后台准备等待，不能当作
  GPU backpressure 次数。stale publication 为 0。
- 后台 preparation：11 个样本，p50 23.037ms、p95 37.124ms。
- 主线程 Metal encode/submit：14 个样本，p50 0.188ms、p95 1.180ms。
- glyph/embedded atlas upload：30 个样本，p95 0.319ms。
- drawable acquisition：14 个样本，p95 0.915ms，运行在 acquisition worker。
- Swift submit attempt p95 38.362ms，包含整个协调器调用及 Instruments 开销，
  不能替代实际呈现间隔；主线程仍有输入准备与 publication 后续工作，尚未完成性能收尾。
- 此轮没有连续触控板 live-scroll 手势，因此呈现间隔样本为 0、p95 为 null。
  **连续滚动 p95 ≤ 16.7ms 尚未验收。**

统计程序按真正的 `MTLDrawable.presentedTime` 排序，按 surface 与已结束手势分组，
不把 busy/跳过调用当作呈现，不拼接闲置间隔，也不剔除长帧。独立测试覆盖乱序回调、
不同窗口、闲置间隔与无样本行为。

本轮回归汇总：`yu-storage-ffi` 44/44、`yu-workspace` 45/45、
`yu-render-macos` 10/10（另 2 项原有 ignored），指标统计测试 3/3。
在正常权限的 macOS 会话重跑 `run-self-checks.sh`，全部 14 项通过，包含此前
在受限会话失败的 clipboard/document-workflow。这仍不替代外部应用剪贴板和
真实 IME/VoiceOver 的人工矩阵。

后台 publication 现在可在 drawable busy 期间复用；只有 GPU 真正提交过当前
publication 的序号，才能被判定为 current。资源完成即使不改变 FrameKey，也不会
因沿用旧的 on-screen key 而丢失提交重试。worker 输出携带独立可消费的 atlas 页，
丢弃旧输出不会使下一帧缺页；接收后台结果还检查最新滚动位置的 coverage。

### 2026-09-11 输入捕获、单 worker 与 publication 校验收尾

本次仅处理以下三个代码问题，不把连续滚动性能和人工矩阵计入完成范围：

1. `EditorRenderSnapshot` 捕获共享的只读文本存储、选区、composition、搜索和
   viewport 配置。正常后台路径不再在主线程调用 `clone_for_render`、重解析
   Markdown 或探测 viewport；worker 重建自己的文档，并从最终 scene 返回可见
   block 信息。等待同一请求期间立即返回 busy，不重复捕获输入或查询资源状态。
2. 每个 host 保留一个 preparation worker。mailbox 最多包含一个待处理请求和
   一个已完成结果，另有一个正在执行的请求；新请求覆盖待处理请求并使旧结果
   失效。resize、字体/scale 变化及 detach/rebind 均复用该线程。worker 自己创建
   CoreText shaper、维护 frame builder 和 CPU atlas，owner 销毁不 join。
3. request sequence 与资源完成代数分开记录。接收 publication 前分别验证
   Revision、FrameBuildKey、request sequence、surface generation、binding
   generation、资源完成代数及最新 viewport coverage；图片 publication/intrinsic
   快照也必须与构帧时一致。新进入视口的资源即使命中缓存而没有完成通知，仍会
   触发重新准备。同步诊断和后台路径交替时，publication serial 连续递增。

取消是协作式的：正在执行的 CoreText/构帧调用可以结束，但过期结果不得进入
当前 host。操作系统拒绝创建 worker 时保留原有同步降级路径，并记录
`worker_fallback`；主线程仍负责资源 cache 收取、结果校验和最终 Metal 操作。
这些边界不等于主线程所有工作均已消除，也不证明长文档准备耗时已达标。

回归覆盖不可变输入在 owner 继续编辑/销毁后的有效性、阻塞构帧时 1,000 个请求
只执行首个与最新请求、detach 清除 pending/ready、销毁不等待、独立代数失效、
同尺寸 surface 替换，以及同步 1x/后台 2x 交替。实际后台 integration 同时断言
canonical editor 没有发生 viewport measurement。

本次验证：`yu-editor` 99/99、`yu-storage-ffi` 48/48、`yu-workspace` 46/46
通过；debug app 重新链接成功，正常图形会话的 Dark Aqua 真实窗口检查和全部
14 项 shell self-check 通过。硬件仍为 Apple M1 Max（Mac13,1），macOS 26.5，
Xcode 26.6；窗口 surface 为 900×692pt、2x，检查包含扩大后恢复及重新绑定。
原始窗口日志保存在本地 `.notes/macos-frame-worker-self-check.log`。

本次 debug 自检采样仅作链路证据：输入捕获 11 次，中位数 0.061ms、最大
18.119ms（计时包含首次字体/host 初始化）；后台 preparation 中位数 25.905ms、
最大 126.785ms。Metal encode/submit 14 次，中位数 0.154ms、最大 1.062ms，
GPU submit/complete 各 14 次，`worker_fallback` 为 0。这不是 Instruments
长文档连续滚动验收；Swift submit attempt 最大仍有 42.638ms，协调器其他
主线程工作及长文档整体开销仍需后续性能分析，不能只用 Metal 段计时宣称达标。

### 后续长文档与延迟资源自动验证

新增可重复的真实窗口回归入口和 Instruments 模式。首次检查中，延迟资源通知、
pending retained reuse、窗口关闭/重开通过，324 KB 长文档的末行导航失败。
后续已修复高度状态传递、缩放几何同步和导航跟随实测高度，长文档与资源窗口
回归均通过。CPU 采样发现的主线程辅助功能排版成本仍待后续优化。具体复现、
硬件与修复后的证据见 [macOS 自动窗口回归记录](macos-render-regression.md)。
这些通过项不等于连续触控板性能或完整人工验收通过。
