# 第五组依赖与系统能力决策

日期：2026-09-28；5A已结项，5B已接通首批PDF链路、尚未结项。依据用户 v1.2，先复用已有代码；系统 API 与成熟开源同按职责评估，不追求全部原生/全部 Rust。Windows 后置。本记录不等于第五组完整分发许可审计。

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

## 5B 选型与首批结果

| 能力 | 本批决定与实际边界 |
| --- | --- |
| 文字与PDF编码 | 采用公开 CoreText/CTTypesetter/CTLine 与 CoreGraphics PDF context。系统负责字体回退、整形和PDF编码；Yu只补纸张、按行分页与内容关系。普通正文保持文字层，没有整个页面栅格化。 |
| 静态SVG | 复用已冻结且安全验证后的helper资源；公开 NSImage 绘制进非屏幕 Quartz context，关闭其显示缓存。原生6页探针中4个SVG页分别含8/8/21/19条路径且无位图；不能由这些样本推定任意SVG无损转换。未使用私有CoreSVG API。 |
| 语义与表格 | 使用5A整文档writer的已解析、安全结构，通过既有HtmlFragment/HtmlTableGrid形成短流程数据；不导入浏览器、不新增Markdown解析器。行内字体与图形由系统度量，完整跨行合并组的分页为Yu特有约束。 |
| Rust候选 | 核对 krilla 0.8.2、krilla-svg 0.8.1 包及源码commit `3ffdf0588cf98050aad6edba51ca70162e1fb5b5`；存档包校验与根LICENSE/NOTICE。NOTICE中的resvg/MPL来源尚未完成文件级授权沿革核对，未复制其代码、未链接进Yu，也未构建Rust候选PDF。原生能力已通过本阶段实际探针，按v1.2 3.3不为凑对照重做第二后端；这不是双后端同语料性能比较或候选完整许可放行。 |
| 界面与文件安全 | 原生NSSavePanel及PDF纸张配置；复用5A固定任务、资源准备、取消、告警同意和目的地事务，不添加PDF专用的不安全直接覆盖。 |

新增直接使用的是工作区已锁定的 `serde_json 1.0.151`，用于Rust到原生的有界流程数据；Cargo.lock只增加一条yu-export依赖边，没有新增package/version。`AppBundle/Resources/PDFExportLicenses` 保存其保守非dev/工作区feature统一闭包的15个包、28份原始许可，加manifest/README共30份文件；可能包含非当前最终链接组件。没有上游补丁、第三方代码或字体复制。该目录实际随Release应用复制并逐字节核对；HTML/helper既有声明继续保留。

采用原生路线不代表字体嵌入和所有阅读器行为自动通过。系统/用户字体文件不单独分发；实际PDF字形、复制、链接、图片和逐页复核继续随5B验证，完整发行target/features、资源/字体义务在5E核对。测试端PDFKit、PDFium及可选PyMuPDF只形成相应测试证据，不引入产品转换依赖。

原始材料位于 `artifacts/group5-5b-20260928-r01/third-party/`，公开依据为Apple Quartz PDF guide、Core Text Programming Guide和上述Krilla固定commit的NOTICE/源文件；具体产物、哈希与后续边界见 `mac-group5-pdf.md`。系统打印和PNG未实现，继续按5C/5D顺序推进。
