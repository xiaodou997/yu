# ADR 0029：隔离 macOS CoreText 字体目录与 fallback 适配

## 状态

已接受（Phase 1）

## 背景

`yu-font` 已经定义了平台无关的 `FontRequest`、coverage/fallback、`TextShaper` 和
`GlyphRun` 契约，但这些契约不能用 `MockShaper` 证明 macOS 上真实字体的 family 选择和
fallback 行为。CoreText 是 macOS 的系统字体服务，适合先用于发现可用 family，并验证一段
包含中文和 emoji 的文本能获得实际的 family/PostScript name。

同时，CoreText 对象属于平台资源。若把 `CTFontRef`、Core Foundation 所有权或 Objective-C
句柄放入共享编辑器状态，会让 `yu-font`、layout 和 `EditorDocument` 被 macOS 生命周期与
线程规则绑定，破坏跨平台边界。

## 决策

新增 macOS-only crate `platform/macos/yu-font-macos`：

- `CoreTextFontCatalog::system` 调用 `CTFontManagerCopyAvailableFontFamilyNames`，复制成排序、
  去重的 `Arc<str>` family 快照；
- `CoreTextFontResolver::resolve` 使用 `CTFontCreateWithName`/`CTFontCreateForString` 的
  CoreText 选择路径，返回请求 family、实际 family、PostScript name、size 和是否 fallback；
- CoreText 的返回对象只在适配器函数内存活，公共结果是拥有所有权的 Rust 元数据，不暴露
  `CTFontRef`、`CFStringRef` 或其他原生句柄；
- 依赖和代码均通过 `cfg(target_os = "macos")` 隔离，非 macOS 构建返回明确的
  `UnsupportedPlatform`，不要求共享 crate 链接 CoreText。

此阶段只验证字体目录和 fallback 选择，不实现 glyph shaping、line breaking 或 rasterization。
真实 shaping 仍通过 `yu-font::TextShaper`/`GlyphRun` 边界接入，且不能改变 source/visual 坐标
协议或进入 `EditorDocument` 的 canonical state。

## 结果

- macOS 可以在真实系统字体目录上运行 live resolver 测试；测试文本覆盖中文和 emoji。
- 共享编辑器核心保持平台无关，未来可用同一元数据边界替换为 DirectWrite、Fontconfig 等实现。
- CoreText 的 FFI unsafe 代码集中在一个小型适配器内，便于后续加入句柄缓存、shaping 和
  accessibility 相关审计。
- 该 crate 不等于产品字体渲染完成；下一阶段需要在同一隔离边界上产生带 source cluster
  range 的 `GlyphRun`，再由 `yu-layout` 消费。
