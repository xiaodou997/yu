# 第六组：macOS 发布收尾（6A → 6F）

## 当前交付状态（2026-09-30）

用户后来选择先提交 Mac App Store；版本标签和 Windows/macOS 联合 GitHub
Release 等 Windows 就绪后创建。下面的 6A–6F 表格和 DMG 实施记录保留为
当时的直接分发计划，当前 App Store 提交以
[商店检查表](mac-app-store-draft-20260929.md) 为准。

直接分发路线的最终 `0.1.2 / build 3` 已完成干净源码构建、公证和软件验收。
本机 `artifacts/releases/0.1.2-3-final-r2/release-manifest.json` 的阶段是
`ready-to-publish`；提交为 `af50c40d0bac7290a5ad2e0de72ae9d2cba1aae4`，
最终 DMG SHA256 为
`ff1bdafacdc5da2ac1af3e8f2314bea6fb64d5903067a9e71403cfd12319aa61`。
安装/保存重开、外部 AX、软件恢复、升级和失败保全的报告均保留在
主工作区 `artifacts/group6-final-acceptance/`。这些证据对应当时的 DMG。

国际化和沙盒修复后的 App Store 正式包来自提交
`ec726f9905a82c7a045fe03804fc44362c49ec29`，仍为 `0.1.2 / build 3`。
正式包的完整清单、Apple Distribution 签名、helper 沙盒继承、Hardened
Runtime、安全时间戳和安装包签名已复核。真实系统打印到 PDF 通过；三种
商店语言对应的资源已包含在五语言包中。提交包已生成，Apple 验证/上传、
构建关联和审核提交仍待完成；商店资料保持草稿、免费及审核后手动发布。

更新：2026-09-29。范围依据用户决定：**实机验收使用 macOS 27 / Apple Silicon，不以 macOS 26 专机作为本组阻塞项。** 最低部署目标仍为 26.0；这不等于已经验证 26 的运行兼容性。用户随后将真实注销、重启及同类会打断本机使用的系统场景移出第六组结项门槛；本组聚焦软件和发行包，不调整非必要系统设置。第五组结论保持原有范围。

## 交付路线与完成条件

首版采用 **Developer ID Application → Apple 公证 → DMG → GitHub Releases**；先实现可靠人工更新。暂不增加 PKG、系统服务或自制自动更新器。第六组主要负责交付，编辑器仅修复验收发现的数据安全、可访问性和系统兼容性缺陷。

按当前范围，整组完成必须有同一正式提交、版本/build、完整包清单、签名身份、Apple submission ID/结果和最终 DMG SHA256；最终包须通过本机安装、打开/编辑/保存/重开、软件异常退出恢复、A→B 人工升级及外部 AX 接口验收。签名、公证、安装和软件行为分别记录，不能互相替代。真实 Gatekeeper 首启、VoiceOver 人工朗读、OS 注销/重启、全新用户环境及 CI runner 均如实标记为未实测或后续跟踪，不作为本组结项门槛，也不记作通过。

| 批次 | 实施内容 | 关闭条件 | 当前状态 |
| --- | --- | --- | --- |
| 6A | 冻结身份、Release 入口、全新包目录、ICNS、版权和许可、构建记录 | 干净提交构建；版本唯一；记录工具链及全部包文件哈希 | 候选构建及发行入口通过；干净提交正式构建待冻结 |
| 6B | 复用本机 Developer ID；helper 先签，app 后签；最小权限 | 两个代码目标均为正确 Team、runtime、secure timestamp、无额外 entitlements；签后原生自检通过 | 候选 app/helper 正式签名、验证与签后自检通过 |
| 6C | app ZIP 公证/staple，再制作 DMG 并公证/staple；保留日志 | 两次 Accepted、两类 ticket 有效、codesign 与静态系统策略检查通过 | app/DMG 均 Accepted、staple 通过；默认 Gatekeeper 首启未实测 |
| 6D | 本机安装、打开/保存/重开、移动/覆盖与卸载数据边界 | 最终公证包在现有 macOS 27 上完成实际应用流程，文档和恢复副本不丢失 | 空文档 blocker 已修复；本机镜像/复制/打开保存重开通过；待最终包复核 |
| 6E | 外部 AX 范围几何、语义与软件级恢复 | AX 查询正确且不改文档；异常退出、损坏记录和多窗口恢复有可核查证据 | 隔离应用外部 AX 跨行/表格/双向几何及语义树通过；最终签名包待复核 |
| 6F | A→B 人工更新、失败保全、下一版本地发行复用 | A→B 升级、退出重开和数据保全通过；从干净提交可重复生产下一版 | A 0.1.0→B 0.1.1 的本机人工升级/草稿恢复通过；CI 和自动更新后置 |

CI 额度/runner 未恢复不阻塞本地签名、公证和本组结项；不能把“本地能跑”记作 CI 已通过。CI 自动化记录为外部依赖，恢复后独立关闭。

## 6A 发行合同

- Bundle ID：`io.github.xiaodou997.yu`，沿用已有身份，保持设置和恢复数据连续。
- 首个候选版本：`0.1.0`，build `1`；之后每次对外发布递增 build，同一版本/build 不替换下载文件。
- 主程序 `Contents/MacOS/Yu`；helper `Contents/Helpers/yu-document-renderer`。
- arm64 only；最低 26.0；锁定现有 Xcode 27 build 27A266a / SDK 27 / Rust 工具链与 Cargo.lock。
- ICNS 从已批准的 Yu.png 生成；Yu Apache-2.0 许可随包携带；已有字体、HTML/PDF 和 324 项 helper 依赖声明继续经过审计。
- 不承诺签名包逐字节可复现：安全时间戳、公证票据和磁盘镜像会改变字节。要求构建输入受控、每份产物唯一可追踪。
- `--candidate` 允许工作区未提交，只用于开发验证。正式构建必须干净 checkout；不能把候选包冒充某个干净提交的正式发布。
- 发行目录必须全新；主程序、helper、Metal 与所有包文件的哈希在每阶段记录。后续阶段先检查包未被改动。

## 本地入口

```sh
# 正式构建：工作区必须干净。开发验证可显式追加 --candidate。
python3 tools/release-macos.py build --output artifacts/releases/0.1.2-3

python3 tools/release-macos.py sign --output artifacts/releases/0.1.2-3 \
  --identity 'Developer ID Application: SHIYU FENG (V6M88BQG7C)' \
  --team V6M88BQG7C

python3 tools/release-macos.py notarize --output artifacts/releases/0.1.2-3 \
  --profile Yu-notary

# 本机软件验收结果逐项写入 JSON 后，最终核验包并标为 ready-to-publish。
python3 tools/release-macos.py finalize --output artifacts/releases/0.1.2-3 \
  --evidence artifacts/releases/0.1.2-3/software-acceptance.json
```

build 执行 locked Rust test/clippy、fmt、Release 构建及已有资源审计、包身份审计、原生自检。sign 使用 `--options runtime --timestamp`，先 helper 后主包，不用 `--deep` 签名；验证时使用 deep/strict。签后重新运行原生自检，发现 Hardened Runtime 导致的实际回归。

当前 entitlements 合同为空，不添加 JIT、unsigned executable memory、DYLD、disable library validation 或 get-task-allow。任何新权限都须修改发行合同、审计及对应测试。

notarize 先验证 Keychain profile，app 通过后 staple，再封装到包含 Applications 快捷方式的 DMG；DMG 独立签名、公证、staple。最后生成 SHA256SUMS 与 release-manifest.json。状态仍为 `notarized-awaiting-acceptance`，不自动发布 GitHub Release。

每个外部命令保存退出码和日志。submission ID 在等待前写入 manifest；网络等待超时不等于 Apple 拒绝。失败目录保留，不覆盖日志。必要时用保存的 ID 执行 `notarytool info/log`，确认结果后在新目录重新运行完整链路；当 app 已 Accepted、已 staple 且包文件未变，并且尚未提交 DMG 时，可用 `resume-dmg --output 同目录 --profile Yu-notary` 继续封装；失败日志和重试脚本哈希保留。已提交 DMG 或签名中断仍先检查现有状态，不自动重传。Accepted 之外均不得 staple 或标为完成。

本机证书身份已存在并可由 codesign 使用，无需创建、撤销或导出私钥。本轮先只读取钥匙串条目元数据，未找到 login 钥匙串中的公证配置；随后用户亲自完成 Yu-notary 配置，notarytool history 认证成功。不扫描或输出用户其他密码。

### 首次配置公证凭据

开发者本人在 Apple 账号“登录与安全 → App 专用密码”建立公证用途密码，然后在本机交互终端输入：

```sh
xcrun notarytool store-credentials Yu-notary --team-id V6M88BQG7C
```

按提示选择 Apple ID 方式、输入开发者 Apple ID 与 app-specific password；默认向 Apple 验证后保存到钥匙串。密码不放脚本、命令参数、Git、日志或聊天。已有 App Store Connect API key 也可使用，不为本任务默认创建额外密钥。

依据：[Apple Developer ID](https://developer.apple.com/developer-id/)、[Apple 公证准备要求](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)、[自定义公证工作流](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)。

## 6D 安装验收台账

在公证通过的实际 DMG 上逐条记录版本、SHA256、主机和实际结果。D01–D06 中与本机软件行为有关的部分是结项验收；真实下载隔离、全新账号及系统权限组合属于后续观察：

- D01 验证最终 DMG 完整性、签名和公证票据；Finder 打开 DMG，在本机测试安装位置运行。浏览器下载 quarantine 与默认 Gatekeeper 首启未实测时明确记录，不为此修改本机安全设置。
- D02 打开 Markdown、编辑保存、退出、新 PID 再打开，逐字核对磁盘内容。
- D03 中文/空格路径与正常文件权限下的打开/保存；非 ASCII 用户名及系统权限允许、拒绝、取消组合留到用户反馈或专门设备验收。
- D04 从只读 DMG 直接启动的明确行为；应用移动后可再开、helper 与资源定位有效。
- D05 退出旧版后覆盖安装；磁盘空间不足/复制中断不能清除文档或恢复副本；保留旧 DMG。
- D06 删除应用保留用户文档、相对图片资源、设置与未恢复副本；清理残留必须明确哪些数据会被删除。
- D07 公开交付前核查仓库可见性、tag/提交、版本唯一、DMG 与 SHA256SUMS；草稿发布验收通过后再公开。

## 6E 辅助功能与软件恢复验收台账

现有 `DocumentTextView.accessibilityFrame(for:)` 已实现，历史缺口已经发生变化；仍须外部系统契约验收，不能仅凭源码关闭。

- E01 macOS 27 正式包：启动、打开/保存、Markdown、图片、Math/Mermaid helper、HTML/PDF/PNG 和打印面板代表链路。
- E02 外部 AXBoundsForRange：单行/跨行、表格、中文/emoji、双向文字、空范围、超界/溢出范围；屏幕坐标、滚动和选区更新；不因查询改变文档。
- E03 检查正文、heading/list/link/table、光标/选区、工具栏/菜单/弹窗的 AX 语义与键盘可达性。VoiceOver 人工朗读保留为后续观察，不以内部自检冒充真实朗读。
- E04 两个已命名与一个未命名文档，保存 checkpoint 后结束测试应用进程并重新启动；逐字比较恢复内容，重新保存后再打开。
- E05 多窗口归档/解码、再次启动的窗口状态和文档对应关系；真实 OS 注销、重启及“重新打开窗口”系统流程明确跳过。
- E06 测试副本异常退出、原文外部变化/删除、恢复目录部分损坏；有效副本仍能恢复，损坏有提示，不覆盖磁盘新版本。
- E07 自动保存开/关、检查点前后的内容边界、IME 未提交内容；恢复保证最后一次成功检查点，不声称未落盘按键可保全。

本组不执行真实注销、重启、修改 Gatekeeper/系统权限或建立全新用户账号；相应系统行为不写作已验证。用户反馈中若出现可复现缺陷，再针对性补测和修复。

## 6F 更新与长期维护

- F01 用相同 Bundle ID、Team ID 的两个真实版本 A/B，A 中创建文档、设置和未保存恢复副本，退出后用 B 覆盖安装；设置、文档/图片、恢复内容均保持。
- F02 B 正常保存后退出重开；校验新版本号、签名与公证票据。未关闭旧进程时明确要求先退出再更新；默认 Gatekeeper 首启不列入本次通过项。
- F03 下载损坏、错误签名/Team、错误架构、磁盘不足、安装中断都不能被宣称升级成功；不得自动删除原应用及用户数据。
- F04 降级前保留用户数据副本；只有实测旧版能读新状态才承诺降级，遇到不兼容恢复格式保留数据并提示，禁止静默覆盖。
- F05 下一次从干净提交递增版本/build，重跑相同本地命令；CI 恢复后安装匹配工具链、复用命令，凭据放专用 Keychain/secrets，日志不含密钥。CI 实际运行单独跟踪。
- F06 自动更新作为后续决策，先比较成熟框架与 Rust/AppKit 接入、更新包签名、公钥管理、失败恢复；首版不因缺少内置自动更新阻塞交付。

## 记录原则

第五组的 778 项 Rust、19 项原生自检等是历史基线；本组按实际本次结果记录，不把既有通过项算成本组系统验收。测试材料继续留本机 artifacts，不提交大型截图、DMG、私钥或公证凭据。

## 2026-09-29 首批实际记录

- 本机 macOS 27.0（26A428），正式证书 Team V6M88BQG7C 已存在；未申请新证书或导出私钥。
- 新发行入口的 7 项门禁测试通过：拒绝开发/ad-hoc/错误 Team/缺失 runtime/缺失 timestamp 签名、危险 entitlement、阶段间包篡改、额外符号链接及脏工作区正式构建；Gatekeeper 被禁用时不得标为强制策略验收通过。
- 首次候选构建在既有 `revision_change_cancels_a_running_child_without_waiting_for_its_output` 中失败：3 秒内未观察到模拟 helper 的 started 标记。保留原始失败记录；未改测试或跳过该项。单独复查通过，随后第二次完整 workspace 测试通过。根因未确认，不把复跑通过当作已定位原因。
- 第二次候选：Rust **1605 passed / 0 failed / 7 ignored**（按全部 test result 汇总），fmt/clippy 通过；构建资源审计通过。已有 7 项 ignored 不计入通过数。
- 签名前后各 **19 项原生 self-check** 及 Bridge 资源失败路径 **2 项**通过。
- app/helper 均为 Developer ID Application、Team V6M88BQG7C、runtime、安全时间戳、空 entitlements；helper 先签，app 后签；deep/strict 验证通过。
- 设置 YU_TEST_RENDERER 指向正式签名 helper，真实公式/图表及过期请求拒绝 integration 测试通过。
- 本机证据：`artifacts/releases/group6-candidate-20260929-r2/`；首次失败目录为 `artifacts/releases/group6-candidate-20260929/`。候选来源是基线提交加本轮未提交变更，manifest 明确 candidate=true，不宣称为干净提交发行。

### 公证和安装冒烟结果

- app ZIP submission：`777fa912-8a73-424f-8b11-e8f0586d25c9`，**Accepted**，Apple 日志 issues=null；app stapler staple/validate 通过。
- DMG submission：`7886f7a5-9573-4404-ba89-d3bd32c01cb3`，**Accepted**；DMG staple/validate 与签名检查通过。
- 最终带票据 DMG：`Yu-0.1.0-1-arm64.dmg`；SHA256：`b5f0a0e0911e50c5bea0c62f0237de08f6f2b130a704cb2222709418899f589d`。Apple 日志里的上传哈希是 stapling 前的文件，不能拿来冒充最终下载哈希。
- **本机 Gatekeeper 已处于 assessments disabled**，不是本任务关闭的。spctl 返回 Notarized Developer ID，同时返回 override=security disabled；不能计作默认策略首启通过。syspolicy_check distribution 返回 0，仍不替代启用 Gatekeeper 环境的实际下载首启。
- DMG 校验通过；只读挂载含 Yu.app 与 /Applications 快捷方式；复制到本机私有中文路径后逐文件哈希/权限一致，签名和票据仍有效。没有冒充干净 Mac 或系统 Applications 安装。
- 复制后的正式签名 app 完整 `--launch-window-self-check` 返回 0；包含源码切换、双向文字、3 档缩放表格/剪贴板/保存重开及帧调度。
- 普通启动参数打开专用中文文件，实际窗口粘贴 Unicode、保存、完全退出、新进程重开，内容逐字一致；这是粘贴，不是实际中文 IME 验收。CUA typeText 首次中文注入仅落入标点，改用标准粘贴后重新验证完整内容。
- **新发现发布 blocker：首次启动新建空白文档、⌘N 新建空白文档均显示“文档暂时无法显示（Rust status 21）”。原有第五组 ad-hoc 包也可复现，不能归因为 Developer ID 或 Hardened Runtime。** 非空文档运行和窗口自检通过不能关闭它。打开面板本轮还观察到所选测试文件的 Open 按钮不可用，按实际未完成保留；通过启动参数打开不替代该面板验收。
- 当前 manifest 为 `notarized-awaiting-acceptance`、candidate=true；未创建 tag、未公开 GitHub Release、未覆盖用户安装的应用、未执行系统注销或重启。

### 2026-09-29 下一批与结项范围调整

空文档 status 21 修复、原生打开/保存面板和 A→B 草稿恢复已完成本机验证，见 [软件收尾验收](mac-group6-software-acceptance.md)。已公证的上一候选版本为 **0.1.1 / build 2**；包含 AX 修复的下一正式构建目标为 **0.1.2 / build 3**。本轮工作区中既有无关 `MindLoci-auth-check/` 不纳入提交或发行。

1. 完成外部 AXBoundsForRange 几何与语义检查；仅修复真实发现的软件问题，并复核软件异常退出和升级失败时的数据保全。
2. 冻结源码提交与版本，在干净 checkout 重跑本地构建、测试、签名、公证；用最终 DMG 完成本机安装、打开/保存/重开和 A→B 升级复核，记录唯一 manifest 与 SHA256。
3. 真实 OS 注销/重启、默认 Gatekeeper 首启、全新用户账号、VoiceOver 人工朗读和 CI runner 明确列为本次未实测或后续跟踪，不阻塞本组按上述软件范围结项；出现真实用户问题时按缺陷处理。
