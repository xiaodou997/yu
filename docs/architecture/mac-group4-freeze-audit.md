# 第四组冻结构建与综合实窗／资源审计

更新：2026-09-26。**最终受测候选为 RC3；已完成本页列出的16组实窗检查及有限时长资源审计，但第四组仍未无条件结项。** 真实跨午夜、多日睡眠／时区切换、完整长时压力与余下组合矩阵没有执行，不能用注入式单测或代表性窗口替代。

测试文档总目录见 [mac-group4-test-index.md](mac-group4-test-index.md)。机器可读证据、逐组断言和截图哈希见 [audit-record.json](evidence/group4-freeze-20260926-rc3/audit-record.json)。没有修复或使用 GitHub CI，没有提前开发导出。

## 1. 冻结身份

| 项目 | 本轮实际值 |
| --- | --- |
| 冻结标签 | `group4-freeze-20260926-rc3` |
| 冻结源码提交 | `6afb7a3dac785d2674236cff1fab65f3546fee39` |
| 源码 tree | `09b21c10c1409d04e9860bf2cfbd4a6fd26243d1` |
| 应用主程序 SHA256 | `ba4caa17724a5813a5c81d208311bd30a42683035773c8fec8187a3b52df539d` |
| 随包 helper SHA256 | `f74c4f402bf005eee852db83f272de35c512d43bffb5e868c84da32ca769f68e` |
| Metal shader SHA256 | `b5f1563b90031cdf588fd00166136fbdcda9dba94df5751324142f46c7b9bbb7` |
| 构建与测试机 | Release、arm64；macOS 27.0（26A428）、Xcode 27.0（27A266a）、SDK 27.0；最低目标 macOS 26.0 |
| 实际产物与日志 | `artifacts/group4-freeze-20260926-rc3/`；保存 `Yu.app`、`source-lock.json`、`build-manifest.json`、`freeze-manifest.json`、`audit-summary.json` |

冻结时工作区干净。后续补充的测试脚本与本审计文档会继续提交 `main`，但**不移动冻结标签、不把文档提交称为已打包源码**。重新构建可能因签名或工具链产生不同哈希；必须重新记录，而不是沿用上表。

真实事件脚本从此 Release 包复制隔离应用并赋予临时 bundle ID、重新 ad-hoc 签名，所以隔离应用主程序哈希可能不同。每组 `results.json` 同时记录原始构建、隔离主程序、未改变的 helper、脚本 SHA256 和完整退出重开 PID。不是把多个不同功能版本混成一个构建。Release 随包／依赖／架构／Metal／ad-hoc 签名检查通过，不代表签名公证或正式分发验收完成。

## 2. 冻结过程中发现并修复的真实缺陷

初始功能基线为 `29643df0`。全仓格式检查发现 `image.rs` 与 `embedded_source.rs` 两处既有换行差异，单独修正为 `70b54992`，形成 RC2。初始1539项 Rust 测试及 RC2 多项窗口检查虽通过，**RC2 最终被拒绝**：

向含 `$x^2$` 的 Markdown 表格粘贴原生合并格后，文件中确实保留公式 span，但实窗把 `<span data-math-style="inline">x^2</span>` 显示为普通原文；完全重开也没有为它启动公式 helper。

原因不是 TeX 丢失，而是生产宿主使用 `equation_source()`，其 `EquationIndex` 只收集 Markdown 语法树中的公式。此前投影测试直接调用 `embedded_source()`，没有覆盖生产索引的归属检查。补的两个测试首先稳定失败：`Formula no longer belongs to this document revision`。

修复提交 `a8ff7794` 将源范围明确的 HTML 公式并入同一索引，按源码排序、去重，继续使用既有编号／引用和错误规则，不绕过校验。新增回归覆盖转换、撤销、重做、BOM／换行重建，以及实体单次解码与全文公式标签引用；先失败后通过。最终 RC3 全仓包含12项表格公式集成测试。

| 实际截图 | 结论 |
| --- | --- |
| [RC2 失败截图](evidence/group4-freeze-20260926-rc3/math-before-rc2.png) | span 原文可见，不能以源码保留断言代替公式显示通过 |
| [RC3 浅色修复](evidence/group4-freeze-20260926-rc3/math-after-rc3.png) | 转换后单元格显示原生 `x²`，不再回退为 HTML 原文 |
| [RC3 深色修复](evidence/group4-freeze-20260926-rc3/math-after-rc3-dark.png) | 同一修复在深色中通过；另外执行真实点击、TeX 正文修改、撤销／重做 |

旧候选与失败日志保留在 `artifacts/group4-freeze-29643df0-rc1/` 和 `artifacts/group4-freeze-20260926-rc2/`，尤其是 `html-math-production-repro.log`、`promotion-light-final/results.json`。不覆盖为成功记录。

另有测试设施失败已单独处理：首次窗口操作触发前台安全检查并中止；脚注脚本在异步滚动前读取坐标，造成命中偏移；合并格选择需要真实拖选而非两次重置选择；拒绝粘贴弹出的原生警告必须先确认，再检查原重做分支。这些通过保留前台校验、等待可见稳定 AX 边界、真实拖选及确认警告修正，没有放宽产品断言。

## 3. RC3 核心与原生检查

| 检查 | 实际结果与边界 |
| --- | --- |
| `cargo test --workspace --locked` | **1541通过、0失败、5忽略**；日志 `workspace-tests.log`，不是复用第五批853项 |
| `cargo fmt --all --check` | 通过；初始失败记录另保留 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过，没有放宽 lint |
| Release 应用产物审计 | 通过，包含随包 helper 与 Metal；`build-release.log` 与 `build-manifest.json` |
| 当前 Release 的 `run-self-checks.sh` | **19项原生检查全部通过**；另外运行 Swift 日历7组、调度器、打字机几何、系统拼写和图片文件系统检查。日志 `native-self-checks.log` |
| 两类样本准备器 Python 回归 | 本轮16项通过；输入生成96份列表与76份表格样本（44成功、32拒绝），默认状态仍为 `not_run` |
| 随包 helper 全部客户端检查，显式含忽略项 | 本轮**6通过、0忽略**，包含真实60秒空闲退出／重启。该命令在 RC1 副本执行；经 SHA256 验证，RC1、RC2、RC3 helper 逐字节相同。不能把它说成 RC3 整个应用测试 |

全仓5项默认忽略包括：helper60秒用例、两个底层 AppKit／Metal live probe、诊断性 spec report 和 fuzz soak。helper60秒已单独执行，其余不冒充通过；当前应用本身的真实窗口与资源流程另见下节。

## 4. RC3 真实窗口矩阵

以下是选定的**16组、164条脚本检查记录**；不是164个独立 Rust 测试函数，也不把重复重跑累加。每组都使用真实隔离应用和外部输入，执行保存、完整退出、新 PID 重开、继续输入及历史恢复。保存文件按 BOM／CRLF 精确比对；所有存储编码排列另由核心测试覆盖，不能把窗口的单一编码组合说成全覆盖。

| 分组（目录位于 RC3 根目录） | 浅／深色 | 覆盖 |
| --- | --- | --- |
| `math-extensions-light/dark` | 各通过 | 行内／块、aligned/cases/多行/CJK公式、上下标／高亮／有限HTML、脚注重复引用／回链、缺失定义诊断与修复 |
| `diagrams-light/dark` | 各通过 | 七类图表逐一插入，实际修改可见标签、撤销重做，公式／图表错误菜单与修复 |
| `recent-diagrams-light/dark` | 各通过 | 三种中心连接／生命周期组合、四种日期时间／Gantt样本；修改一个可见标签，不改参与者身份 |
| `html-light/dark` | 各通过 | 合并格编辑与导航、表格扩容／字面转义、表格图片粘贴与替换、HTML列表 Enter／Backspace／缩进 |
| `promotion-click-light/dark` | 各通过 | 原生 Option+Shift 拖选 donor，真实复制粘贴；公式／脚注表格转换、合法跨组、两个真正跨界拒绝；公式鼠标命中与 TeX 编辑、转换后脚注 Command-click 定义；拒绝后旧重做分支仍有效 |
| `ime-light/dark` | 各通过 | 系统拼音输入源，实际硬件键输入 zhongwen、空格提交“中文”、一次撤销重做、第二次组字 Escape 取消，最后恢复原输入源；不是 Unicode 粘贴冒充输入法 |
| `list-levels-light/dark` | 各通过 | 固定列表样本中的8个单选区场景（独立列表／单元格各4）：父子一起缩进、跨层级退一级、根级／div组合；实际 Cmd+[ / Cmd+]、精确源码与主 AX 选区历史 |
| `resource-lifecycle` | 浅色通过 | 普通文档不启动helper、共享按需启动、真实空闲退出与回收、缓存画面保留、编辑后新PID重启、应用退出释放 |
| `resource-cancel` | 浅色通过 | 暂停本测试自己的helper响应后修改文档，取消旧进程与迟到资源；普通文本替换不误启，新合法资源恢复 |

`promotion-light/dark` 是较早的成功跑法，最终表格矩阵采用包含真实点击编辑的 `promotion-click-*`，不重复计数。脚本各版本的 SHA256 都在结果中；其后新增独立测试旗标不修改冻结二进制。

列表8个单选区通过不等于96个排列全跑，更不等于多主选区／反向选区的全部实鼠操作；这部分有核心回归但窗口覆盖仍按实际记录。表格5个代表性场景也不代替76份 manifest 全部执行。准备器清单保持原 `not_run`，本轮实际执行的子集在 `audit-record.json` 中独立列出。

## 5. 资源回收与图像复核

RC3真实应用的空闲观察，从完成初始渲染／测试设置后开始计时，**55.55秒后观察到helper退出**，不是把其60秒空闲策略改成55秒。采集50组 `proc_pid_rusage RUSAGE_INFO_V4` physical footprint 样本：应用 `77,874,208..81,052,704` 字节，helper `4,866,456` 字节；退出后明确验证旧进程被回收，新编辑使用新PID。

受控取消场景在**0.683秒**内观察到旧响应进程被取消（计时包括输入事件，不是纯渲染性能基准）。它只暂停测试自己启动的helper，不触碰用户的其他进程。16组完整退出重开均校验旧helper释放；最终进程复查没有发现本轮隔离应用／helper残留。

这些是有限样本的生命周期／回收通过，**不是长时泄漏证明或所有资源峰值上界**。高频、多文档、长期连续开关与睡眠恢复压力仍须补证据。

实际查看了 RC3 **57张截图的12页联系表**，覆盖浅深色公式、七类图表、近两批图表、合并表格／图片、转换公式脚注、跨层级列表、输入法提交与取消恢复。源图路径和 SHA256 在机器记录的 `visual_review` 中；复核仅针对捕获的可见区域，不能推断屏外内容。`central-1` 比窗口高，当前所审截图未覆盖其最上端；深色下黑色原始logo对比度低，未进行图像改色，不冒充所有截图的逐像素或完整长页验收。

## 6. 结项前尚缺的证据

| 未执行范围 | 不能用什么代替 |
| --- | --- |
| 前台静止跨真实午夜、多日睡眠后唤醒、系统时区／时钟／会话变化组合 | 不能用 Swift 注入日期7组或 helper 的 `reference_day` 变化代替真实系统通知；没有修改生产机器系统时钟 |
| 长时高频资源／多文档压力与峰值 | 不能用50组短窗口足迹或一次取消证明无泄漏 |
| 完整列表／表格UI排列、多选区／反向鼠标选择、合并表格真实列宽拖动 | 本轮代表性场景、核心几何和原生协调器自检不能替代这些操作 |
| `group4-smoke.md` 整文档目录／details跳转与所有屏外区域联合复核，以及全部既有写作流程复跑 | 分离专项通过不等于整份综合文档、所有相邻功能已经同场景通过 |

因此当前状态是：**明确构建已冻结；本轮执行矩阵与有限资源审计通过；实窗发现的公式生产缺陷已修复；第四组最终结项保留。** 下一步补这些证据，不再重复实现已经通过的基本链路，也不把第五组导出提前插入。

## 7. 重现与材料入口

标签锁定产品源码；最新 `main` 提供补充后的测试脚本与本审计。分别记录源码版本和执行脚本 SHA256。不要在已有证据目录上覆写。

下列先建立独立源码工作树，但不切换当前目录；后面的测试命令从含最新脚本的 `main` 仓库根目录执行。`--ime`、`--list-inputs` 与增强点击检查属于冻结后测试提交，不在旧标签脚本中。跨机器重现时，将新构建放入实际运行脚本的 `HERE/.build` 并重建正确 manifest，或者把审计版测试脚本复制到已完成构建的冻结工作树中再运行；这类测试文件变化须单独记录，不回写已冻结产品源码。

```sh
git fetch origin --tags
git worktree add --detach ../yu-group4-rc3 group4-freeze-20260926-rc3
# 在该工作树按原 build-app.sh --release 流程打包并记录新的二进制哈希。
# 执行本轮脚本时，核对其 HERE/.build 的包就是要测的构建。
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 tools/prepare-group4-list-checks.py artifacts/group4-next/list-inputs
python3 tools/prepare-group4-paste-checks.py artifacts/group4-next/paste-inputs
python3 platform/macos/yu-shell-macos/run-embedded-checks.py --recent-diagrams --reopen artifacts/group4-next/recent-light
python3 platform/macos/yu-shell-macos/run-embedded-checks.py --promotion-suite --reopen artifacts/group4-next/promotion-light
python3 platform/macos/yu-shell-macos/run-embedded-checks.py --ime --reopen artifacts/group4-next/ime-light
python3 platform/macos/yu-shell-macos/run-embedded-checks.py --list-inputs artifacts/group4-next/list-inputs --reopen artifacts/group4-next/list-light
```

最近七批全部验收说明、可直接打开的样本、13份列表源语料及表格载荷对应关系统一见 [测试文档索引](mac-group4-test-index.md)。
