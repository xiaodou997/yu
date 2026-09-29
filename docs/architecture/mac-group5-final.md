# 第五组 5E：统一构建验收与正式结项

更新：2026-09-29。依据第五组 v1.2 及用户后续确认：测试资料留本机，非阻塞体验细节后续按用户反馈处理；内容完整性、原文/历史安全、文件安全和任务可恢复性仍是硬要求。

## 结论

**第五组通过 5E，并正式结项。** 5A HTML、5B PDF、5C 系统打印软件链路、5D PNG 均已分别结项；5E 又在同一冻结 Release 上统一重跑四条原生链路和四条真实用户入口，完成跨格式内容一致性、24 组总台账以及最终依赖/许可证/字体资源审计。X01—X08、H01—H04、F01—F06、T01—T03、I01—I03 共 **24/24** 关闭。

本结论不包含实体打印、macOS 26 专机、发行签名/公证、安装/更新、VoiceOver 或 CI 环境；这些属于第六组发布收尾。Windows 仍后置。不增加 Word/EPUB，不引入 Pandoc 或浏览器生产导出引擎。

## 冻结构建身份

- `Contents/MacOS/Yu` SHA256：`e26de688cdf3c16d511ae4d9a2ab3c264574056a48272c3327e7c31b0ea800ae`
- `yu-document-renderer` SHA256：`f74c4f402bf005eee852db83f272de35c512d43bffb5e868c84da32ca769f68e`
- `yu_shaders.metallib` SHA256：`b5f1563b90031cdf588fd00166136fbdcda9dba94df5751324142f46c7b9bbb7`
- Release / arm64；最低部署目标 macOS 26.0；SDK 与本轮受测主机 27.0。
- 5E 工程收尾提交：`5dca3c7f`；从该已提交源码重新构建后主程序 SHA256 仍为上述 `e26de688…`，证明最终受测二进制与提交后的产品源码一致。

HTML、PDF、系统打印、PNG 的最终原生 suite 与最终真实窗口 suite 共 8 份 build manifest 均绑定上述同一主程序 SHA256。最终 `verify-macos-app.py` 审计通过，包含架构、最低系统、签名、系统动态依赖、浏览器运行时拒绝、字体/许可资源一致性和 helper 依赖声明检查。

## 5E 实际回归

- `cargo test --workspace --locked`：**778 项通过，0 失败**。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`：通过。
- `cargo fmt --all --check`、`git diff --check`：通过。
- macOS headless self-check：**19 项通过**；Bridge 资源失败路径 **2 项通过**。
- 最终 HTML 原生链路与真实窗口链路：通过。
- 最终 PDF 原生链路与真实窗口链路：通过。
- 最终系统打印原生链路与真实打印面板链路：通过；**实体打印任务 0**。
- 最终 PNG 原生链路与真实窗口链路：通过。

5E 不重新制造 5A—5D 已完成的全部压力笛卡尔积；10 次双文档交替、3 次取消/重试、边界/失败保护、独立浏览器/Preview 等证据沿用各批结项记录。5E 的职责是把代表性生产链路重新绑定到同一最终构建，并检查跨格式和发行资源没有在后续批次中漂移。

## 跨格式一致性

- HTML：14 项公式/图表、1 张本地图片，共 15 项内嵌资源；目录、脚注、折叠正文、重复标题与文档末尾标记存在。
- PDF：5 页；14 项公式/图表、1 张图片；固定中文句可搜索；目录/脚注链接对象仍存在。
- 系统打印另存 PDF：5 页；未发送实体任务。去掉各自测试专用的未保存标记和页码后，文字层与直接 PDF 输出一致。
- PNG：浅色 1× 综合文档为 800×4034，可解码；5D 已另外签收 2×、深色、编号分段、跨段合并表格和不可分割超大元素拒绝。

跨格式正文标记按 Unicode 文本、仅忽略排版产生的空白比较；该检查是**语义一致性**，不是要求 HTML/PDF/打印/PNG 像素级排版相同。

## 依赖、许可与字体资源

5E 发现并关闭一个真实发行声明缺口：helper 通过 `typst-assets 0.15.1` 的 `fonts` feature 内嵌字体，原包此前只有精选 `FontNotices.txt`，没有随包完整保留 `typst-assets` 自身 NOTICE。最终构建已补 `TypstAssets-Apache-2.0.txt` 与 `TypstAssets-NOTICE.txt`。完整 NOTICE 包含 Libertinus、DejaVu/Arev、Foxit、NewComputerModern（含 NewCM10-Regular 的 GPLv3 + Font Exception + Distribution Exception）等实际资产条款。

新增可重复的 `tools/generate-native-renderer-licenses.py`：基于锁定 `aarch64-apple-darwin` Cargo metadata 计算 `yu-document-renderer` 实际依赖闭包，最终记录 **324 个外部 package**，全部有 Cargo license expression；其中 31 个发布包根目录没有独立 LICENSE/COPYING/NOTICE 文件，清单明确保留该事实而不伪造文本。可获得的 516 份许可/NOTICE 类文件被汇编进 `RustDependencyLicenses.txt`，最终文件 2,754,345 字节；`RustDependencies.json` 记录包名、版本、来源、features、许可表达式和文件哈希。此为工程分发审计，不替代法律意见。

应用主包的 6 个 Open Sans 静态字体继续固定到来源 commit、逐文件 SHA256 和 SIL OFL 1.1；`verify-macos-app.py` 现在强制核对 `Fonts`、`HTMLExportLicenses`、`PDFExportLicenses`、NativeRenderer 声明和锁定 Cargo 清单，避免以后依赖变化后仍沿用旧声明。

## 失败记录

失败没有覆盖为成功：

1. 5E 首次 HTML targeted suite 因 Runner 给 Python 的 PATH 中没有 Cargo，12 个 core 子项被执行器记失败；显式使用锁定 Rust 工具链路径后同 suite 通过，产品输出未因此修改。
2. 第一次跨格式文字标记比较因 PDFKit 在中文顿号后插入布局空格而失败；改为仅去除布局空白的 Unicode 文本比较后通过，同时 PDF 与系统打印仍要求去掉测试标记/页码后全文一致。
3. 最终 HTML 实窗一次在打开第二文档时，AX 仍读到正在退出的 OpenPanel/前一文档；测试夹具调整为等待目标窗口存在并先 raise 后读取正文。随后另一次未命名菜单点击出现单次系统事件未落地；未改生产逻辑，再次完整运行后 IME、保存覆盖、未命名资源基准和 10 次交替全部通过。

## 24 组总台账

- 公共：X01—X08，**8/8 passed**
- HTML：H01—H04，**4/4 passed**
- PDF：F01—F06，**6/6 passed**
- 系统打印：T01—T03，**3/3 passed**
- PNG：I01—I03，**3/3 passed**

合计 **24/24 passed**。机器汇总保存在本机 `artifacts/group5-5e-20260929-final/reports/group5-final-ledger.json`；跨格式汇总为同目录 `cross-format.json`。这些 artifacts 不提交仓库，也不向用户发送复核附件。

## 后续

第五组到此停止开发和验收，不继续扩张导出格式或追加组合测试。下一任务组为第六组“发布收尾”：macOS 26 实机、VoiceOver/系统恢复、发行签名与公证、安装分发、更新机制，以及 CI 环境恢复后再接持续反馈。
