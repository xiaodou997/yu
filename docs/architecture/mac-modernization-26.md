# Yu Mac 26+ modernization

2026-09-18：用户确认仅支持 macOS 26.0+ / Apple Silicon；Xcode 27 release / SDK 27。本文取代历史 mac-native-v3 中的 macOS 14、Intel、Universal 和 Typora 像素复制要求。共享 Rust 内核保持平台独立。

## Delivery contract

- One native AppKit shell and one CoreText/Metal editor; no old-shell switch.
- System toolbar/sidebar materials; document canvas is opaque.
- Yu default theme (16pt, 26pt target line height, maximum text width 760pt, minimum gutter 24pt), Github/Night optional.
- CoreText resolves system font roles; actual fallback face identity is retained through rasterization.
- Product builds reject an incompatible Xcode/SDK/architecture before Cargo runs. Deployment target is 26.0 throughout Swift, Rust and Objective-C.
- Shader IR is compiled at build time. The app passes its bundled metallib location to the session; invalid/missing resources are errors. Rust standalone tests use the same precompiled shader build output, not a runtime source compiler.
- Evidence binds executable hash, build configuration, toolchain, minimum OS, architecture, shader resource, runtime and test results.

## Work in progress

The implementation is not yet accepted. Written code must not be confused with tested delivery.

| Stage | Current work | Required evidence |
| --- | --- | --- |
| N0 | arm64 / 26 targets, strict toolchain preflight, removal of universal build | Clean release artifact audit, CI toolchain pin |
| N1 | NSSplitViewController/sidebar item, customizable NSToolbar, segmented navigation, find accessory | Window tests and safe-area/candidate geometry; source-mode action and light/dark window checks passed; real IME remains pending |
| N2 | Yu light/dark, shared theme identity, native system font roles, optional classic themes | CoreText contract, theme switching, Chinese/RTL, snapshots |
| N3 | View-bound display link, precompiled Metal, adjacent batch encoding, reusable GPU-completed buffers, bounded image decode/cache | Pixel/behavior regression, memory pressure, measured before/after performance |
| N4 | Pending real interaction and restoration checks | Actual Chinese IME and mouse operation, save/reopen, 26 and 27 runtime |

## Acceptance

Use Yu-owned snapshots at 900x620, 1200x800 and 1600x1000, light/dark, sidebar/find open and closed. Check clipping, overlays, spacing, jumps and consistent hit/caret/IME coordinates. Typora reference matching is no longer a gate.

Existing P1 manual checks remain pending; this platform change does not turn them into passes. The original binary baseline was removed on 2026-09-18 at the user’s explicit request to clear historical Mac build outputs. Its artifact manifest and earlier acceptance records remain; do not claim a fresh before/after runtime comparison without reconstructing that baseline.

Targets (not current measurements): input-to-presentation p95 <=32ms; 60Hz dropped frames <1%; 1MiB resize visible update p95 <=100ms; unfocused settled idle CPU <=0.5%; physical footprint <=150MiB/250MiB for 100KiB/1MiB plain documents. Test 120Hz, low power, cross-display moves and cancellable 5MiB documents separately. Glyph atlas 32MiB, decoded-image cache 128MiB; record transient/in-flight and GPU allocations independently.

Metal 4, math/diagram/export completion remain separate iterations. Do not declare macOS 26 or real input accepted without actually running those checks.

## 2026-09-18 clean rebuild validation

Historical Mac outputs were removed at the user's request; Rust upgraded to 1.98.1, Tree-sitter to 0.27.0 and Comrak to 0.55.0. Final release identity and evidence are in `artifacts/mac-modernization/validation.json` and `build-manifest.json`.

The rebuilt release passes artifact auditing, 1,026 Rust tests (4 ignored), 14 native self-checks and light/dark actual-window suites on macOS 27. Twelve Yu-owned screenshots cover all three target window sizes, both appearances and sidebar states at 2×. The zero-size viewport found by the window suite was fixed by embedding the complete NSSplitViewController view hierarchy. Image-cache upsize requests now replace inadequate decoded thumbnails without losing intrinsic dimensions.

This validates the rebuild iteration, not all N0–N4 requirements. Source-mode UI/projection, real IME/mouse acceptance, restoration/VoiceOver, measured power/memory/latency, macOS 26 runtime and remote CI remain outstanding. The public runner release-toolchain limitation remains documented in the dependency audit.

## 2026-09-18 source-mode delivery

阅读／源码切换已接入工具栏与 ⌘⇧M，使用同一 Rust 源码、历史、CoreText 排版及 Metal 绘制。源码模式保留字面语法、等宽字体、空行和表格分隔符；大纲标签仍按语义显示。模式变化进入帧身份并拒绝旧后台结果。

本轮最终 Release 的记录位于 `artifacts/mac-source-mode/validation.json`：1,033 Rust 测试通过（4 忽略）、14 原生自检和浅深色真实窗口套件通过。源码模式截图、候选框坐标、跨模式历史及 BOM/CRLF 保存均有自动化证据。实际中文输入、完整窗口恢复/VoiceOver、性能与 macOS 26 仍待验收；上节的“source-mode UI/projection 尚未完成”仅代表之前的构建时点。

## 2026-09-18 window experience and resource checks

窗口归档增加侧栏宽度、反向选区和挂载窗口前的源码锚点恢复；源码模式切换保留后台 worker，内存压力重建保持发布序号递增。浅深色归档重建、查找焦点和 AX 源码检查串行通过。详见 `artifacts/mac-window-experience/README.md` 和绑定构建身份的 `validation.json`。

隐藏空闲测量：100KiB 为 113.28MiB / 0.0216% CPU；1MiB 为 426.91MiB / 0.0118% CPU，内存未达 250MiB 目标。脚本滚动/缩放行为通过，但首开和重开各一次 AX 独立排版使总门槛失败。N3/N4 仍未完整验收；归档重建不代替系统重启，隐藏空闲不代替可见失焦，AX API 不代替实际 VoiceOver。

## 2026-09-18 bounded paragraph cache and pre-surface AX

段落缓存改为按最近使用顺序淘汰，限制段落数量、几何项和视觉文本；前后台合并同样执行限制，全文数值高度独立保留。生产协调器在 surface 就绪前不查询 AX 几何，避免尚无窗口时触发独立排版。新增淘汰后高度/快照/源码一致性回归。

最终构建 `1ca2254a…` 的 1,034 Rust 测试、14 原生自检、浅深色编辑/归档恢复及严格滚动回归通过，AX/hover 独立排版计数均为零。1MiB 稳定 footprint 降为 106.69MiB，隐藏空闲 CPU 0.0081%，该场景通过；100KiB 内存为 90MiB，但采样中窗口重新激活，空闲门槛未通过。详见 `artifacts/mac-memory-ax/README.md` 和 `validation.json`。

因此上一节的长文档内存超标及 AX 回归失败已解决，N3/N4 的完整性能/人工体验仍未验收。段落预算允许单个超大活动段落及在用快照额外占用；不要将它描述成进程内存硬限制。

## 2026-09-18 actual presentation timing and scheduling

增加当前文档/几何绑定的 Metal `presentedTime` 查询，替换仅返回呈现布尔值的接口。新脚本测量原生文本输入/撤销及字号更新到实际匹配帧呈现，分别保存操作、呈现和轮询观察时间，不能当作物理键盘/IME 的端到端数据。

加入排版/drawable 就绪唤醒、首个视口优先、单个呈现后后台细化任务、字形页指纹缓存、CoreText 字体目录及前后台上下文复用；drawable 改为 2，保留垂直同步。最终构建 `bac86fa1…` 的输入 p95 为 49.24ms（基线 65.88ms），仍未达到 32ms；1MiB 字号缩放 p95 为 81.12ms（基线 165.10ms），通过脚本化 100ms 门槛。

1,036 Rust 测试、14 原生自检、浅深色编辑/归档恢复及严格滚动回归通过。1MiB 隐藏空闲为 110.05MiB / 0.0058% CPU，约 61.7 秒帧序号不变。完整性能验收仍未通过；最终证据见 `artifacts/mac-presentation-latency/README.md` 和 `validation.json`，不要以中间构建更低的样本替代最终数据。

## 2026-09-18 presentation wait investigation

移除每帧重复写入 surface 可见性，增加诊断模式的实际呈现策略记录。运行时确认垂直同步开启、双缓冲、非事务呈现。主要等待在 GPU 完成到显示之间（诊断中位数约 20.6–37.5ms），尚不能归因于单个系统组件。

最终构建 `77db5e9c…` 的输入 p95 59.32ms、撤销 48.87ms，仍失败；1MiB 字号缩放 81.22ms，通过。中间输入结果在 32.67–49.15ms 波动，未证明稳定优化。14 原生自检和浅深色窗口回归通过；详情及所有中间证据见 `artifacts/mac-input-presentation/README.md`。完整性能验收仍未通过。

## 2026-09-18 input CPU and display-cycle diagnostics

辅助功能桥接改为复用上次容量并直接填充，增长时才重试；删除刷新路径的重复语义树读取。Swift 容量增长/缩减的节点、标签、索引和版本回归通过。诊断中辅助功能同步耗时中位数由 3.27ms 降至 1.98ms，输入至提交由 10.23ms 降至 9.19ms。

最终构建 `093e25b3…` 的三轮输入 p95 为 48.73 / 49.18 / 49.26ms，均未通过 32ms；1MiB 字号缩放 p95 81.62ms，通过。14 原生自检、专项容量检查及浅深色窗口套件通过。显示周期诊断只定位等待，不计入性能门槛；详见 `artifacts/mac-input-cpu/README.md` 及 `validation.json`。

## 2026-09-18 Metal system trace and retained presentation control

取得系统级 Metal 轨迹，48 次操作的命令/显示事件与应用 presentedTime 一致。新增 `--case redraw`，在源码和 revision 不变时呈现已有帧，对照 p95 在 32.5–49.3ms 间变化；它不是输入性能通过判据。独立清屏探针存在首帧不稳定，仅保留实验，不纳入验收。

打包改为原子替换可执行文件，修复原地覆盖签名产物后架构检查被终止的问题；连续重建哈希一致。最终 `f1fa275e…` 的输入/撤销 p95 为 32.66/32.17ms，仍未通过 32ms；字号缩放 81.64ms，通过。14 原生自检和浅深色窗口回归通过。详情见 `artifacts/mac-metal-timeline/README.md`，完整性能与 N4 仍待验收。

## 2026-09-18 文档与数据安全交付

第一组已完成新建、另存为、系统最近文档、自动保存、恢复副本发现及恢复、外部冲突保护、多窗口关闭/退出。未命名文档首次保存采用原生面板；另存为保留源码、BOM/CRLF及历史；恢复沿用原磁盘基线，手动确认前不自动写回。恢复延后与损坏副本均有保护，应用退出取消不提前删除恢复记录。

最终 `b10e4327…` 的 1,043 Rust 测试通过（4 忽略），14 原生检查、浅深色窗口回归和文件流程通过；独立进程的最近文档持久化与 SIGKILL 后三类恢复通过。详见 `mac-document-lifecycle.md` 与 `artifacts/mac-document-lifecycle/validation.json`。原生面板注入决策，真实 IME、VoiceOver、macOS 26 和系统注销/重启仍未验收；本轮不改变性能指标结论。
