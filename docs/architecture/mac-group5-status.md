# 第五组状态：5A 首批 HTML 链路已实现，验收仍进行中

更新日期：2026-09-28。

## 当前结论

第五组已启动。5A 已有真实「文件 → 导出 HTML…」菜单、原生配置/保存/进度/取消/警告控制器，以及固定快照到便携 HTML 的生产链路；不是空接口或仅生成 fixture。本轮完成代码、Release 构建、定向核心/原生任务检查和实际 HTML 产物。

**5A 未结项，第五组未结项；完整验收组关闭 0/24。** 已通过的命名子项不可替代真实菜单交互、独立文件直接打开和有限重复/资源观察。PDF、系统打印、PNG 尚未实现，没有添加占位菜单。下一步先收尾 5A，再按 5B → 5C → 5D → 5E 推进。

## 源码与构建身份

- 开工基线：main `2cd81829e7ac53b1bfa7a93980a9cc390bf5e387`。
- 实现提交：`506473d4cf30a88d69b57d2412f104a1c4f434c2`，`feat: add snapshot-based portable HTML export for group5 5A`。
- 最后受测 Release 主程序 SHA256：`534825ed5a01850577e643a0a0116a7727d4f7d36752eecb6d434789cba2d01c`。
- helper：`f74c4f402bf005eee852db83f272de35c512d43bffb5e868c84da32ca769f68e`。
- shader：`b5f1563b90031cdf588fd00166136fbdcda9dba94df5751324142f46c7b9bbb7`。
- 产物最低系统字段：macOS 26.0；本次构建 SDK 27.0。不是 macOS 26 专机全量验收。
- 本机证据：`artifacts/group5-5a-20260928-r03/`，源码定向检查在 `artifacts/group5-5a-20260928-finalchecks/`。r01/r02 与首次失败日志保留。
- r03 的 `source-lock.json` 对候选源码逐文件记录 hash，包括当时尚未提交的新模块；`source-commit.json` 已将 115 份受测范围文件逐字节核对到上述实现提交。文档更新不改变该产品构建。

已有无关未跟踪目录 `MindLoci-auth-check/` 未修改、未暂存。构建、导出文件和私人原始日志不默认提交仓库。

## 本批改动

Rust 复用 comrak、Yu 公式/脚注/TOC/有限 HTML 模型与资源路径规则；新增整文档 writer、资源冻结和目标身份事务。AppKit 使用 NSSavePanel 和原生进度/警告界面，ImageIO 用于图片方向归一与 PNG 编码，现有 helper 提供固定日期/样式/revision 的静态 SVG。未引入 Pandoc、浏览器生产引擎或第二个 Markdown 权威。

HTML 内嵌样式、已解析本地图片、公式/图表；保留重复脚注回链、稳定目录锚点、展开 details 和安全元数据。主动/未支持内容转义并告警，远程图片默认不下载；用户明确确认后才提交 completed_with_warnings。文档导出安全合同与旧剪贴板 HTML 路径分开。

只读任务快照可包含未保存编辑；存在 preedit 时拒绝启动，不代为提交/取消。资源先固定字节，检测准备期间变更；禁止覆盖源文件、图片及符号/硬链接别名。同目录临时写入并检查后发布，取消与提交由明确门控排序。窗口关闭取消所属任务，不借用其他文档的 helper 缓存。

初始公开预算：正文 8 MiB、单本地资源 32 MiB、冻结图片总量 128 MiB、内嵌资源输出累计 64 MiB、最终 HTML 256 MiB；图片 32 Mi 像素，SVG 4 MiB/100000 XML 节点，资源最多 2048 项，同时最多 2 个任务，准备阶段 300 秒。不据此宣称已完成所有预算边界或性能验收。

## 实际运行结果

| 检查 | 本轮结果 | 证据边界 |
| --- | --- | --- |
| `cargo fmt --all --check` | 通过 | 格式检查 |
| `cargo clippy --locked -p yu-export -p yu-storage-ffi --all-targets --no-deps -- -D warnings` | 通过 | 指定两个包，不是全工作区 Clippy |
| `cargo test --locked -p yu-export -p yu-markdown` | 249 项通过 | 含既有回归；不是 249 个实窗用例 |
| `tools/test_group5_export.py` | 3 项通过 | 新 suite/产物检查器入口测试 |
| Release 构建与原有包审计 | 通过 | FFI 92 函数/29 类型一致；真实应用二进制已生成 |
| `group5-export` r03 | 定向检查通过 | yu-export 43 项（包含在上面 249 项中，不累加）；12 命名 core 子项、6 native 子项、5 产物结构子项 |
| 原生任务场景 | 浅色、当前主题代表/BOM+CRLF、已有 redo 分支、告警确认、取消保留旧输出、取消后重试均通过 | 通过产品 bundle 的 `--html-export-self-check`；不冒充菜单点击/实窗手势 |
| 许可文本随包 | 69 个文件逐字节一致 | 67 份原始许可文本 + manifest + README；不是完整发行法律审查 |

正常输出为 14 项静态公式/图表（含七类图表）和 1 张本地图片，0 警告。原生检查覆盖本例 preedit 拒绝、磁盘原文不变、revision/选区/dirty/history 不变、未保存快照包含且后续编辑不混入；history 场景实际执行了原有 redo 分支。

固定异常语料按预期返回 6 项诊断：错误公式、错误图表、不安全 HTML、危险链接、远程图片、缺失图片；任务结果为 `completed_with_warnings`。取消场景保留既有输出。不得将此解释为支持范围内有效内容可以随意降级。

## 实际 HTML 与独立查看

r03/outputs 下含 `light.html`、`dark-bom-crlf.html`、`history.html`、`warnings.html`、`retry.html` 及任务报告。取消样本保留 OLD-OUTPUT-KEEP，不当作 HTML 产物。

浅色 HTML：83857 字节，SHA256 `686fe1b93a48888c72ebe4523fd7e8f208703dd6bf158f2f2d6b1a5b0ee75f2e`。

深色/BOM+CRLF HTML：SHA256 `1d360ade4c973d021f21958b4a7121c557a27500e30096141e5380528caf5532`。

在独立 Linux Chromium 144.0.7559.96 中对浅色文件的**原样字节**进行离线渲染，15 张内嵌图像均 complete 且尺寸非零，0 脚本、0 外部请求、0 页面异常、0 悬空片段链接，details 初始展开。已查看顶部、公式/表格、图表中段和文末脚注截图；普通正文、公式编号、合并表格和文末内容可见。

**限制：**测试环境管理员策略拒绝 file:// 导航，因此该次复核使用浏览器 set_content 载入未改写的产物字节，不属于“移动文件后用真实浏览器直接打开”的完成证据。原始 HTML 未因测试改写；其 hash 与 r03 相同。完整截图/报告保留在本对话提供的浏览器检查资料中；Mac Safari/其他外部查看器的直接打开仍待补验。深色样本已有 native/结构证据，本轮未做独立浏览器视觉签收。

## 验收台账与下一轮

X01—X08、H01—H04：`partial`，仅上述实际命名子项 passed。F01—F06、T01—T03、I01—I03：`not_run`。24 组明细及逐子项证据在 r03/reports/cases.json。

5A 下一轮先串行完成真实菜单/保存面板、主题选择、未命名文档资源基准、IME 拒绝、覆盖确认、进度/取消/再次导出和关闭所属文档；补文件移动、Yu 退出后在外部浏览器直接打开、断网资源完整和深色视觉检查。然后按固定范围执行连续 10 次导出、至少 3 次取消/重试、两个文档交替，并记录内存/helper 生命周期。补齐未覆盖的权限/写失败/资源变化/预算边界；只针对发现的具体缺陷扩展回归，不开启无界组合验收。

本轮未运行整个 `run-self-checks.sh`、全工作区 test/Clippy、真实菜单自动化、长期资源观察、macOS 26 专机验收，也未运行 PDF/打印/PNG 链路。没有为本组重开第四组已关闭范围，没有修复 CI/签名公证/更新分发。

首次 Rust 可见性/Swift actor 隔离编译问题、测试 unwrap lint 已修正并在新构建/定向检查复测。许可采集首次发现 registry 不含 .cargo-checksum.json，改从已锁定 Cargo.lock 记录包校验；原始失败不覆盖成功重跑。第三方选型/归属与仍需最终核对的范围见 mac-group5-dependency-decisions.md。
