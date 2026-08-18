# Phase 3：Source Projection & Native Layout

## 目标

在不复制 Markdown source、selection、IME 或 history 的前提下，把 Rust 的 source-backed
projection、layout 和 hit-testing 接入 macOS native editor。阶段初期只建立 Revision-bound
FFI 与诊断边界；当前已进入最小可见 RenderPlan overlay，但 TextKit 仍保留为输入、IME、
Accessibility、caret/selection 和失败回退表面。完整 visual mirror 仍需后续逐步替换。

## Track A：Projection bridge

- [x] `DocumentEditorSession` 暴露 inline projection 的 owned snapshot
- [x] macOS storage FFI 提供 expected-Revision projection UTF-8 count/fill
- [x] macOS storage FFI 提供 source UTF-16 ↔ visual UTF-16 caret round-trip
- [x] Swift/AppKit projection self-check 覆盖 strong/emphasis/link delimiter 与 Unicode caret
- [x] macOS storage FFI 按 parser-owned block index 暴露 source range/kind/visual lengths 与 UTF-8 snapshot
- [x] Swift block projection self-check 覆盖 heading/task/fenced-code、Unicode 和 stale/out-of-bounds
- [x] macOS storage FFI 暴露 generation-bound composition projection、visual selection 与 marked caret
- [x] composition projection metadata 暴露 visual replacement range，供 visual IME overlay 绑定
- [x] Swift composition projection self-check 覆盖 Unicode preedit、update stale generation、cancel/source 保持
- [x] 建立 source-backed GFM table projection kind 与 UTF-16 cell-range count/fill ABI
- [x] table visual projection 只保留 source-backed cell runs，隐藏 pipe、cell 空白、row line ending 与 delimiter physical row，并覆盖 source↔visual mapping
- [x] `TableLayoutSnapshot` 隐藏 delimiter physical row，按 metrics 生成 source-backed cell geometry，并提供 Rust/FFI hit-test count/fill 诊断契约
- [x] shaped table layout 按 projection visible runs 测量 cell 宽度，并把 source-backed cluster/glyph 定位到 column/row/alignment；cell-aware caret/hit-test 在 hidden boundary 返回可见 cell source boundary，内部位置保持 projection bias
- [x] 表格 visible cell address 固定为 header=0、body 从 1 开始并跳过 delimiter；Editor Tab/Shift-Tab 按 source cell row-major 顺序移动 caret，不改变 Revision/Undo，首尾无目标时返回 Unhandled
- [x] macOS storage FFI 暴露 Revision-bound table column/row divider resize hit-test（kind/index/局部 position）；outer edge、非法 tolerance、非 table block 和 stale Revision 拒绝，实际 drag transaction 后置
- [x] `yu-layout::TableResizeGesture` 固定按下/移动/释放/取消的 Revision-bound、source-neutral 状态；document-host self-check 消费 column/row hit ABI 并验证 tolerance、source 不变和 stale 拒绝
- [x] GFM table resize 第一版采用 session-only column geometry：transient layout 保持总宽度、最小列宽和 source ranges，不进入 layout cache；row geometry 等待 variable-row contract
- [x] storage FFI/Swift block-projection self-check 接入 session-only column geometry count/fill；canonical layout、source 和 stale Revision 边界均验证
- [x] `yu-workspace` 将 caller-owned column override 接入 viewport scene/render plan；table border/fill、cell glyph 和 render command 共用 transient layout，stale/row/source/cache 回归覆盖
- [x] storage FFI 将 table resize 推进为 session-owned begin/update/finish/cancel；macOS CoreText-shaped begin 与 retained frame 共用 shaper/font size，Swift self-check 覆盖 preview→frame→finish→cancel
- [x] heading/blockquote/list 与 task/fence 复用 parser-owned structural prefix，统一 block projection kind tag；列表 bullet/task 文本仍保持 source-visible
- [x] visual selection range、metrics hit-testing 和 point↔source mapping 的 Revision-bound 诊断契约
- [x] stale Revision/generation 在 native projection callbacks 上的全路径回归（含视觉 scene/glyph/render-plan count/fill header）

## Track B：Native layout

- [x] parser-owned block-scoped projection snapshot、惰性 layout metadata 与 block-local caret
- [x] macOS CoreText shaping metrics 与 `yu-layout` line/caret contract 对齐
- [x] shaped viewport snapshot、block origin/height 与可见窗口 count/fill
- [x] storage FFI shaped viewport header 暴露 scroll/viewport/max-scroll 坐标协议
- [x] storage FFI shaped caret scroll target 与 visual viewport transform self-check
- [x] TextKit 过渡镜像自检支持 visual/source 双向映射（生产 view 尚未切换）
- [x] `DocumentTextView` visual pointer adapter 与 source-mirror fallback self-check
- [x] `DocumentTextView` opt-in visual IME composition mirror、marked range 与 attributed substring self-check
- [x] 生产 visual view 启用点击/拖选 visual boundary→Rust source selection，以及 source→visual caret 映射
- [x] 生产 visual view 的 projected selection highlight 与同一 Revision 的 shaped caret reveal
- [x] 产品窗口接入独立 visual decoration sibling，负责 selection/caret 绘制并在 stale/detach 时回退 TextKit
- [x] decoration sibling 改用 Rust/CoreText-shaped document-space count/fill geometry；active composition 使用 generation-bound transient Rust layout，查询失败时保留 TextKit fallback
- [x] 生产 Up/Down/Shift-Up/Shift-Down 使用当前 CoreText metrics/shaper 的 Revision-bound command
- [x] 生产 pointer adapter 使用同一 CoreText-shaped Rust block layout 命中 visual boundary；TextKit 只保留输入/IME/AX/矩形回退
- [x] macOS table divider hover 复用 document-space shaped hit-test，显示/清理 resize cursor；hover 不创建 session，finish 前后 source 保持不变
- [x] macOS storage FFI 暴露 viewport 内 Revision-bound table divider Accessibility descriptors；Swift 接入真实 NSAccessibility splitter，VoiceOver increment/decrement 更新 session-only preview，并自检 stale element 销毁与 source 不变
- [x] visual pointer 正向/反向拖选通过 Rust endpoint ABI 保留 anchor/focus 方向，继续拖动和 Shift-click 不丢失 selection 语义
- [x] visual IME active caret 使用 Revision + composition generation-bound CoreText shaped block geometry
- [x] visual IME composition hit-test 使用 Revision + composition generation-bound transient projection，覆盖跨 block document-space mapping
- [x] visual IME preedit 在所属 block 使用 CoreText shaped glyph、CPU atlas 与持久 Metal surface 发布
- [x] visual IME preedit 的跨 block transient layout：按受影响 block span 投影、重测 viewport 高度并进入持久 RenderPlan/Metal publication
- [x] macOS RenderPlan 将 document-space scroll origin 传入 Metal viewport，统一滚动后的 glyph/damage 坐标变换（仍保留 TextKit 字形回退）
- [ ] 完整 visual renderer 迁移（移除 TextKit source mirror 的生产渲染职责）

## Track C：Scene and rendering

- [x] Rust `ViewportSceneInput`/`SceneBuilder` 生成 Revision-bound 最小 owned scene snapshot，macOS host 以 count/fill 自检 primitive 顺序、来源范围、坐标和 stale 丢弃（诊断桥，尚未替换生产 renderer）
- [x] `yu-scene` 消费 `TableLayoutSnapshot` 生成 source-backed header/selection/border `TablePrimitive`，并由 `yu-render` 以 solid-fill command 保持 painter order；`yu-workspace` 在 viewport scene 中先提交 table decoration 再提交 cell glyph（完整产品窗口仍保留 native fallback）
- [x] Rust 使用 CoreText glyph rasterization、CPU `GlyphAtlas` 与 `yu_workspace::assemble_viewport_render_frame` 生成 Revision-bound RenderPlan；macOS host 以 count/fill 自检 glyph command、atlas page fingerprint、damage 和 stale 丢弃（诊断桥，尚未接入生产窗口）
- [x] `yu-render-macos` 新增持久 `CoreTextViewportFrameBuilder`，重复 Revision 重用 CPU atlas/RenderPlan fingerprint；ignored AppKit probe 使用真实 CoreText publication 进入 `MetalAtlas`/retained target（生产窗口仍未切换）
- [x] persistent macOS host 通过 count/fill ABI 暴露 Revision-bound retained glyph primitives（含 atlas placement、metrics、bounds 与 source block range；生产 view 仍未切换）
- [x] macOS document host opt-in surface-submit self-check 使用临时 AppKit `NSView` 完成 `CAMetalLayer` attachment、drawable、atlas upload 与真实 Metal submit（生产窗口仍保留 source mirror）
- [x] persistent native surface adapter 复用同一 view 的 Metal surface/renderer/atlas，覆盖重复提交、resize generation、显式 detach 与 stale Revision（生产窗口仍未切换）
- [x] product `NSView` surface lifecycle coordinator 接入 attach/layout/resize/scroll/edit/close，空文档通过 CoreText metrics FFI 初始化 viewport（source TextKit mirror 仍可见）
- [x] macOS document host 诊断桥持有 persistent CoreText/atlas/publication host；编辑、scroll、resize 的 frame serial、surface generation 和 stale Revision self-check
- [x] macOS native GPU surface 在 ignored AppKit probe 中消费 Rust-owned CoreText workspace publication（生产窗口仍保留 source mirror）
- [x] active composition 的 transient block layout/glyph atlas 进入同一持久 RenderPlan 与 Metal submit，Swift submit key 绑定 composition generation
- [x] heading、emphasis、code、link 的最小真实 visual render 通过产品窗口 persistent Metal
  surface 可见提交；TextKit 仍保留为透明 overlay 下的输入/AX/回退表面
- [x] 当前 Revision、composition generation、submit geometry 和 Rust decoration frame 同时有效时隐藏 TextKit source glyph；编辑、滚动、resize、IME、stale、detach 与 submit 失败自动恢复
- [x] 产品窗口将 source-glyph gate 收敛为带 frame identity 与 fallback reason 的显式 visual render state machine，并提供无窗口 transition self-check
- [x] Rust surface 与 Rust-shaped decoration 成对控制可见性；TextKit fallback 不再与旧 Rust glyph surface 叠加
- [x] `DocumentTextView` 以 source fallback、projected TextKit overlay、Rust surface 三种显式 presentation role 管理绘制责任
- [x] projected TextKit overlay 限制在 active composition；普通 stale/geometry/surface 失败直接回到 canonical source fallback
- [x] active composition 的 Rust surface 与 Rust decoration 共用 transient block layout、caret/selection geometry 和 composition generation；TextKit overlay 仅作为失败回退
- [x] RenderPlan/decoration count-fill 在 composition update/cancel 后拒绝旧 generation，清空旧 header 并要求重新 publication
- [x] source-glyph gate 抽出纯 publication identity predicate，验证 surface、decoration、Revision 和 composition generation 必须同帧匹配
- [x] visual publication 额外要求当前 RenderPlan 含有可绘制 command；空/空白文档保留 TextKit source fallback，避免空 Metal surface 隐藏可编辑内容
- [x] fenced code block 的 Revision-bound `FillRect` 背景进入 Scene/RenderPlan/Metal solid pipeline，并保持 fill-before-glyph painter order
- [x] `yu-projection::ImageSource` 保留 inline/reference image 的 source/label/destination ranges，并随 strictly-outside edit 映射
- [x] `yu-assets::ImageCache` 建立可轮询异步解码队列、destination 去重、RGBA8 校验和 Revision-bound CPU publication
- [x] macOS storage FFI 暴露 source-backed image metadata count/fill；Swift self-check 覆盖 reference resolution、fingerprint 与 stale Revision
- [x] ImageIO 解码 worker、Metal RGBA texture ownership、ready-image RenderPlan command 和未就绪 placeholder（资源级纵向切片）
- [x] image placement 使用 source/alt/visual ranges 生成 document-space layout geometry；Scene/RenderPlan 以 glyph 后 overlay 顺序发布，metrics/CoreText hit-test 返回完整 image source range
- [x] 将 `ImagePublication`/`MetalImageAtlas` 接入 macOS 持久 surface host；snapshot 暴露 image upload/resource 计数，surface self-check 覆盖 ImageIO→Metal ready texture
- [x] image 请求收敛到 CoreText 当前 viewport/overscan block；`ImageCache` 增加有上限的 LRU、Revision-bound 失败诊断，ready publication 在下一帧按真实 intrinsic 宽高比更新 placement bounds
- [x] intrinsic image 高度进入对应 block 的 HeightIndex、content height 与 max scroll；Metal image atlas 在 publication 集合变化时淘汰离屏 texture，snapshot 暴露 atlas eviction 计数
- [x] 图片 intrinsic metadata 与 decoded pixels 分离并跨帧保留；同一 Revision 的失败按有界指数退避重试，仍保留 fallback
- [x] `yu-image-scheduling-bench` 覆盖 2,000/100,000 级图片 block 的 viewport/overscan 请求量与耗时
- [x] `ImageRequestPlan` 按 destination 去重并以 visible 优先排序；macOS surface snapshot 暴露候选、去重、overscan 与 retry 计数

## 约束

1. Markdown source、Revision、selection、composition 和 history 仍只由 Rust
   `DocumentEditorSession` 持有。
2. Swift 不解析 Markdown，不根据 delimiter 自行推导 source range。
3. 任何 visual snapshot、layout、scene 和 glyph cache 都必须携带 Revision；过期结果不得提交。
4. 完整 visual mirror 接入前，现有 native source mirror 必须继续可用，确保 IME、复制粘贴和
   Accessibility 有安全回退路径。
5. visual IME overlay 只能消费同一 Revision + composition generation 的 Rust projected text 和
   visual replacement range；generation 失效时必须回到 canonical source mirror。
6. visual viewport 的 block `y`、caret `y` 和 scroll target 都是同一 Revision 的 document-space
   坐标；Swift 只能使用 header 提供的 scroll transform，不能复制 HeightIndex 或自行推导高度。
