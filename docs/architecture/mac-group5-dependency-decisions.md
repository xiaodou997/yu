# 第五组依赖与系统能力决策

日期：2026-09-28；当前仅落地 5A。依据用户 v1.2，先复用已有代码；系统 API 与成熟开源同按职责评估，不追求全部原生/全部 Rust。Windows 后置。本记录不等于第五组完整分发许可审计。

## 5A 采用的组合

| 能力 | 已有实现 / 候选 | 本轮决定与实际结果 |
| --- | --- | --- |
| Markdown、引用和扩展内容 | 已有 comrak 0.55.0、Yu Markdown/HTML/公式/脚注/目录模型 | 保留 comrak（工作区 default-features=false），新增独立整文档入口；不改剪贴板 unsafe 合同，不添加第二个语法权威。综合样本已生成含表格、混合脚注、静态公式与七类图表的 HTML。 |
| HTML 安全 | 已有有限 HTML 解析/样式白名单；候选 rust-ammonia/ammonia | 未引入 ammonia。当前仅输出 Yu 有限语义，未支持/主动内容整体转义并告警；默认清理不能代替可读源码保留。输出还有限定 CSP。不声称任意 HTML/CSS 浏览器兼容。 |
| 文件目的地 | AppKit NSSavePanel、原有窗口菜单 | 采用系统保存面板；新原生控制器只负责配置、进度/取消/警告交互。面板不代替 Rust 目标身份和资源保护。 |
| 图片 | 已有 yu-assets 路径规则、ImageIO | 固定本地资源字节；ImageIO 解码、按方向转正并编码 PNG；内嵌后不保留本机路径。不另加图像解码引擎。 |
| 公式/图表 | 已有 yu-document-renderer | 按固定 revision/样式/日期请求现有 helper 的静态 SVG，不借用视口旧图、不加入 JS/CDN/外部命令行工具。 |
| SVG 安全检查 | 已锁定 roxmltree 0.21.1 | 直接依赖复用 XML 解析；拒绝 DTD、主动元素/事件、处理指令、外部引用等；只做有限静态资源验证，不实现 SVG 渲染引擎。 |
| 图片内嵌 | 已锁定 base64 0.22.1 | 复用库编码，不复制源码。 |
| 临时文件与发布 | 已锁定 tempfile 3.27.0 | 复用同目录临时文件、persist/persist_noclobber；Yu 补源文件/资源身份保护、提交门控和取消状态。 |

Cargo.lock 仅增加 yu-export 对上述已有包及 yu-assets/yu-syntax 的依赖边，没有新增包版本条目。Rust 包直接声明和实际 features 以本批 Cargo.toml、Cargo.lock、应用构建及记录的 metadata 为准；默认功能并非“无传递依赖”。

## 归属保留

`platform/macos/yu-shell-macos/AppBundle/Resources/HTMLExportLicenses/` 保存直接复用的四个外部包及其保守的非 dev 解析闭包：36 个包、67 份原始 LICENSE/COPYING/NOTICE 类文件，总计 322886 字节（不含清单）。`manifest.json` 记录所选版本、Cargo.lock 包校验、workspace-resolved features、上游仓库和每份声明的 SHA256；无本地上游源码补丁。既有 build-app.sh 已将 AppBundle/Resources 复制入应用包，无需另造许可证工具框架。

该闭包按 aarch64-apple-darwin 的工作区 metadata 收集，可能含 feature 统一与构建阶段组件，不宣称所有包都进入最终二进制，也不宣称它覆盖整个 Yu/helper 分发。5E 仍须核对实际发行 target/features、文件级例外及字体/资源义务。

保留 comrak 完整 COPYING（含继承代码与单列规范测试材料说明），没有复制其测试目录。finl_unicode 和 unicode-ident 的 Unicode 许可原件单独保留；rustix 的 LLVM 例外说明也未删除。双许可包保留原始可选许可文件，不将它们改署 Yu 的 Apache-2.0。此次仅复制许可/归属文本，不复制系统/用户字体、主题或第三方测试素材。

沿用 helper 现有资源与声明；新导出使用其静态结果。字体显示、轮廓化、嵌入与重新分发不可混为一谈，完整产物/发行包的字体与资源核对仍列为 5E 检查；本批不向用户提供字体文件。上游 AI 贡献、CLA/DCO 另行遵守，本轮不向 comrak 自动提交 PR。

## 后续选型边界

5B 做一次有界的 CoreGraphics / 最相关 Rust PDF-SVG 候选比较；可以同时评估，不要求先让系统实现失败。krilla/krilla-svg 的选定版本 NOTICE/MPL 来源必须先核实再引入；resvg/usvg 按真实缺口复用，不将栅格化冒充 PDF 矢量。svg2pdf 不作为新依赖首选。尚未引入这些候选，也未实现 PDF/打印/PNG 产品入口。
