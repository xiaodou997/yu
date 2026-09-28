# Yu 验收脚本

统一入口：

```sh
python3 tools/run_acceptance.py group4-fixed artifacts/新目录
```

`run_acceptance.py` 是所有功能共用的执行器。它创建独立输出目录、串行执行命令、保存原始日志和退出码，并根据必需证据层计算每个用例的状态。第四组的固定输入、预期和日志判定只在 `group4_fixed.py` 中。旧命令 `run_group4_fixed_acceptance.py` 只是兼容入口。

## 新功能接入

在本目录新增 `feature_name.py`，实现 `run_suite(ctx) -> int`。命令行名称用连字符，例如 `feature-name` 对应 `feature_name.py`。一个最小套件：

```python
from acceptance_runner import write_json


def run_suite(ctx):
    code = ctx.run("core", ["cargo", "test", "--locked", "-p", "yu-editor", "new_feature"])
    ctx.ledger.add("new-feature/example", ["core", "real_window"])
    # 先核对测试输出中属于该用例的明确成功证据，再记录本层结果。
    if code == 0 and "new-feature/example PASS" in ctx.log("core").read_text():
        ctx.ledger.record("new-feature/example", "core", "passed", "core")
    rows = ctx.ledger.finish()
    write_json(ctx.output / "reports" / "cases.json", {"cases": rows})
    return 0 if code == 0 and rows[0]["status"] == "partial" else 1
```

套件自己负责生成输入、核对文件哈希、确认每个 ID 实际完成了对应断言，并定义必需证据层。通用执行器拒绝重复用例、重复步骤、重复证据层以及用失败命令记通过；只有全部必需层通过才产生 `passed`。未做真实窗口检查时应保留 `partial`，不要把核心测试的通过标记同时用于窗口层。

真实窗口也用同一个 `ctx.run(...)` 串行启动现有桌面驱动。套件需核对驱动的 `results.json`、用例 ID、截图与保存结果，再用 `ctx.artifact(path)` 保存证据文件的路径和 SHA256，最后记录 `real_window` 或 `cold_reopen` 层。`ctx.run` 不会因为命令返回 0 就自动给每个用例通过。

已有证据层：`core`、`native`、`real_window`、`cold_reopen`、`system_event`、`visual`。套件可针对功能选择需要的层，不必强制所有功能跑同一套组合。输出目录必须是全新目录；已有结果不会被覆盖。

## 第五组 HTML 验收

```sh
# 核心、原生任务与产物结构。先用现有 build-app.sh --release 构建。
python3 tools/run_acceptance.py group5-export artifacts/新的核心目录

# 真实菜单、组字拒绝、保存/覆盖、未命名图片基准，以及两个文档交替10次。
python3 tools/run_acceptance.py group5-window artifacts/新的窗口目录

# 单独的固定1000公式输入，使取消发生在任务运行中；3次取消/重试及所属窗口关闭。
GROUP5_WINDOW_PHASE=cancellation python3 tools/run_acceptance.py group5-window artifacts/新的取消目录

# 剩余告警确认/Escape取消、未选择图片基准、图片不可读和写目录失去权限。
python3 tools/run_acceptance.py group5-safety-window artifacts/新的告警目录

# 资源预算/文件安全核心检查；8个原生成功/失败/取消场景验证完整选区和真实redo/undo。
python3 tools/run_acceptance.py group5-safety artifacts/新的安全目录

# 已公布的正文/SVG/出现次数、输出预分配、任务并发/截止时刻和真实ImageIO像素边界。
python3 tools/run_acceptance.py group5-bounds artifacts/新的预算目录

# 对上述窗口目录中的实际输出制作移动副本，确认生产者已退出，离线直接 file:// 打开。
GROUP5_WINDOW_EVIDENCE=artifacts/新的窗口目录 python3 tools/run_acceptance.py group5-browser artifacts/新的浏览器目录
```

桌面用例必须串行执行，需要已有辅助功能、事件投递和截图权限。使用重新签名的隔离应用和独立状态目录；输入不经过系统剪贴板，结束时恢复输入源。`window` 与 `cancellation` 为有界的两个阶段，不扩展验收组合。普通220公式导出可能在AX定位取消按钮时完成，因此它只承担重复输出，不把提交后的点击当成运行中取消。第一次失败保留，不能用新目录覆盖。

浏览器阶段只使用测试机已有的 Chrome 和带内置 WebSocket 的 Node.js；两者不是产品依赖，也不进入 Yu 生产导出链路。新建隔离浏览器配置、关闭缓存并模拟该页面离线，不改系统网络或用户浏览器配置。只复制并移动产物副本，原始证据文件保持不动。自动检查不自动完成 `visual` 层；必须记录实际查看过的截图、哈希及范围。

`tools/test_group5_window.py` 验证阶段路由与失败证据保留，不是GUI证据。所有这些入口只完成台账中的命名子项，不能自动关闭全部24组。最新结果和仍未解决的原生菜单/候选状态观察见 `docs/architecture/mac-group5-status.md`。

安全套件只对本次新建的隔离图片和输出目录暂时撤销读写权限，并在finally/defer中恢复；不修改系统或用户目录权限，不通过填满磁盘制造错误。原生安全自检在新目录创建自己的fixture，拒绝复用已有目录，记录成功/取消/失败时的源文、版本、脏状态、多选区/方向或表格矩形及历史能力，并实际执行原有redo/undo和继续编辑。辅助入口 `--html-export-safety-self-check NEW_DIRECTORY` 不是实窗证据。

`tools/ime-menu-reference.swift` 是无自定义输入行为的AppKit NSTextView最小对照，不进入Yu产品；用于区分系统/驱动的菜单行为与Yu特有导出缺陷。对照复现不自动成为候选恢复验收通过，也不等同于用户接受风险。

预算套件不抬高生产限额：8MiB源码和4MiB/100000节点SVG实际进入解析，32Mi像素图通过生产ImageIO归一和HTML任务。64/128/256MiB的累计/拼接边界用生产预检函数及低内存重复片段验证，不冒充满载解码/256MiB成品压力。双任务用真实HtmlJob准入与释放；300秒用确定Instant检查前一纳秒/到期分界，不修改系统时钟、不等待五分钟，也不宣称是强制抢占被阻塞的系统调用。新增原生入口 `--html-export-budget-self-check INPUT_DIRECTORY NEW_OUTPUT_DIRECTORY` 拒绝复用已有输出目录。

现有窗口菜单子项还会在移动到文末、两次放大、切换源码模式后重新导出，比较HTML字节和选区不变；另存 `reports/presentation-isolation.json`，不增加验收组数量。缩放/源码模式使用已有原生快捷键，缩放以菜单状态核实，源码模式另看截图；不把AXPress返回成功当作动作生效。

`export_write_failure` 的父测试只在自己创建的子进程中降低文件大小限制，并屏蔽该子进程的XFSZ信号，触发真实write_all失败；核对旧输出与临时文件清理。忽略标记的子测试由父测试显式启动并核对成功标记，不是漏跑；不修改宿主、用户shell或其他进程限额，不填满磁盘。

## 第五组 PDF 验收

```sh
# 使用已审计的 Release，真实快照到原生 PDF；正常/纸张/表格/长代码/告警/立即取消。
python3 tools/run_acceptance.py group5-pdf artifacts/新的PDF原生目录

# 串行桌面检查：真实 PDF 菜单、默认配置、保存取消、中文路径、覆盖取消/确认。
python3 tools/run_acceptance.py group5-pdf-window artifacts/新的PDF窗口目录

# 真实纸张/页码/基准目录、10次双文档交替、3次运行取消/重试和所属窗口关闭。
python3 tools/run_acceptance.py group5-pdf-lifecycle artifacts/新的PDF生命周期目录

# 复用生产任务，PDF专属选区/历史/失败安全及尺寸/文件别名边界；目录必须不存在。
platform/macos/yu-shell-macos/.build/Yu.app/Contents/MacOS/Yu --pdf-export-safety-self-check artifacts/新的PDF安全目录
platform/macos/yu-shell-macos/.build/Yu.app/Contents/MacOS/Yu --pdf-export-edge-self-check artifacts/新的PDF边界目录

# 可选开发端独立产物检查，需要该检查环境已有 PyMuPDF；不是产品运行依赖。
python3 tools/check-group5-pdf.py manifest.json 新的PDF检查报告.json
```

`--pdf-export-self-check INPUT OUTPUT` 为原生入口；`--pdf-letter`、`--pdf-landscape`、`--pdf-warnings`、`--pdf-cancel` 只控制该测试。正常快照包含未保存标记而排除后续编辑，检查原文/选区/版本与继续编辑撤销。表格输入40个两行合并组、长代码140行；源内容完整检查不能取代逐页视觉或阅读器交互。取消用例目前是启动后立即取消，不冒充三次运行中取消/重试。

独立检查清单的 `files` 数组逐项记录 `file`、`sha256`、`kind`（composite/table/long-code/warnings）、`paper`（A4/Letter）、`landscape` 和 `unsaved`。文件位于清单目录内，检查器拒绝已有报告并核对每份字节身份。当前固定语料均使用36pt或44pt页边距和页码；仅在底部35pt区域验证9pt页码后，才将该页码与跨页正文计数分开，原始文字提取结果仍保留。PDFium/PDFKit/PyMuPDF 的文字、绘制与 GUI 证据分层记录。

各 suite 只记录本次命名子项，不自动关闭完整验收组。聚合结论及用户调整后的非阻塞细节反馈范围见 `docs/architecture/mac-group5-pdf.md`，不改写旧 suite 的 partial/失败历史。

`tools/inspect-pdf-native.swift` 只读检查页面、文字、链接与页脚。`tools/pdf-preview-driver.swift` 仅向明确PID的系统Preview和精确匹配的测试PDF窗口投递事件；测试文件须位于本项目artifacts，不能针对其他文档、系统剪贴板或全局偏好执行操作。阅读器搜索/拖选、目录/文末注往返及放大与PDF解析证据分别记录；未执行系统复制或外链启动不能填为已通过。

原生页数边界直接调用生产写出器，1000页通过、1001页拒绝；不是1000页逐页视觉验收。共享任务测试依次执行HTML/PDF两个格式，验证两个任务准入、释放和300秒确定时刻边界，不修改时钟或等待五分钟。桌面测试保持串行；报告和截图留本机，不默认向用户发复核附件。
