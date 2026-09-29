# 第六组：空文档、安装、软件恢复与 A→B 升级

日期：2026-09-29。用户要求软件优先，不调整非必要系统设置。本轮保持 Gatekeeper、系统权限、登录项和系统版本不变，不执行注销/重启，不启用自动更新、不公开发行。

## 结果

**空文档发布 blocker 已关闭。** 本机安装包完整性、原生打开/保存、A 0.1.0 build 1 → B 0.1.1 build 2 的文档/恢复状态保全及新进程重开通过。后续外部 AX 查询又发现并修复跨行范围漏算中间文本、语义节点未暴露给系统两项问题；这些修复已在隔离应用上验证，最终签名包仍待复核。第六组尚未整体结项：升级失败保全和干净提交发行仍待完成。用户已将真实 OS 注销/重启及同类系统场景移出本组门槛；VoiceOver 人工朗读、默认 Gatekeeper 首启、全新用户环境及 CI runner 保留为未实测或后续跟踪，不记作通过。详见 [调整后的发行计划](mac-group6-release.md)。

### 空文档根因与修复

后台排版先执行 `LayoutQuery::Source(focus)`，空文档的合法光标位置为 byte 0，但 Markdown 中没有可供查找的 block。原路径返回 `BlockOutOfBounds`，工作线程将其转换成 `YU_STORAGE_RENDER_HOST_UNAVAILABLE`（21）。同步视口路径本来能生成空白布局，因此只看同步自检会漏掉错误。

`LayoutContext` 现在先校验 source offset；确认为空文档时复用正常空白视口快照。光标、行高、后台发布使用同一几何，不制造段落、不改变源码，不把非法位置当作空白文档。

新增回归先观察到后台路径 Err(21)，修复后通过：

- 编辑器：预览/源码模式的 byte 0 查询成功；1 和 u64::MAX 仍被拒绝。
- FFI：同步和后台两条路径均生成实际帧、单个光标和正行高，源码保持空字符串。
- 真实窗口：浅色/深色各检查空文件和新建未命名文档；预览/源码模式下首次输入、撤销回空、重做、删除全部内容及再次撤销均成功呈现。
- 外部 CUA 操作：普通无参数启动、新建、Unicode 粘贴、撤销/重做、删除全部内容不再出现 status 21。

原生入口：`Yu --empty-document-window-self-check EMPTY_FILE`，深色追加 `--dark-mode-self-check`。只对测试文档注入关闭选择；真实产品退出流程保持原样。

### 辅助功能软件修复

`accessibilitySourceRange` 原先先计算 `NSMaxRange`，恶意或损坏 AX range 的 location+length 可能溢出。现在先检查 location、length 与余量，再访问 NSString；并同时核对当前源码长度。负数、NSNotFound 和 Int.max 附近范围的字符串/几何查询回归通过。

这只关闭范围校验风险，不代表完整 AXBoundsForRange 的跨行/表格/双向几何或 VoiceOver 朗读已验收。现有语义、折叠内容、链接/任务等原生 AX 自检继续通过。

## 候选身份与公证

- `io.github.xiaodou997.yu`；0.1.1 / build 2；arm64；最低目标 26.0，本轮实机 27.0。
- app/helper：Developer ID Application，Team `V6M88BQG7C`，Hardened Runtime，安全时间戳，无额外 entitlements。
- 签后 Yu SHA256：`24f10ecb2b669b721554224cdc422b8eac8622a1cba331426130e9bd8a80d39f`。
- 签后 helper SHA256：`6aed5660028f84fe542e1761b471b392c21bfb2f47262f8f24207018fd5fbca4`。
- app 公证：`365db6f4-d5e0-473b-9a5d-a450416253e1`，Accepted；staple/validate 通过。
- DMG 公证：`b7960504-b713-4d66-a2e4-3b6501707b5a`，Accepted；staple/validate 与签名检查通过。
- 最终 DMG SHA256：`36046348f3f429afb55b346d04f422bc962bb1d9ffcb56490b8dbaa308eb83ed`。

证据目录：`artifacts/releases/group6-0.1.1-2-candidate/`。工作区尚未冻结为干净提交，manifest 明确 candidate=true；没有创建公开 Release 或把候选签名包称为正式发布。

## 安装与升级实际操作

在本机私有 `artifacts/group6-upgrade-a-b/Applications/Yu.app` 安装位置保留正式 Bundle ID 和签名。没有覆盖系统 Applications 中的用户安装，也没有删除用户数据。

1. 复制 A 0.1.0 build 1，打开中文路径文档并产生保存内容。
2. 在 A 新建未命名草稿，输入中文/emoji；确认实际 `.yurecovery` 检查点落盘后，仅 SIGKILL 本次测试进程。
3. 复制并验证 B 的签名/票据，保留 A 为 `Yu.previous.app`，替换应用；文档和恢复目录逐文件哈希与替换前一致。
4. B 普通启动发现旧草稿；通过真实恢复弹窗选择恢复，AX 正文与预期逐字一致，应用要求确认后手动保存。
5. 通过原生保存面板将恢复草稿保存为测试目录内的 `未命名.md`；磁盘字节与 A 原草稿一致。
6. 原生打开面板通过文件项的原生 Open 动作打开 A 已保存的文档，正文与磁盘一致。
7. 完全退出 B，以新进程再次打开保存后的草稿；正文仍完整，状态为已保存、磁盘一致。
8. 最终 DMG 只读挂载校验通过；DMG 内应用、release manifest 和升级测试实际使用的 B 应用逐文件内容/权限一致，Applications 快捷方式指向 /Applications，app 公证票据有效。

这是现有 Mac 上的私有安装目录和人工覆盖升级验收，不等同于浏览器下载隔离属性、干净用户账号、非 ASCII 用户名或默认 Gatekeeper 首启。没有承诺降级兼容性、更新中断的所有故障排列或自动更新。

## 软件恢复与窗口状态

使用可指定 `--app` / `--build-manifest` 的原生 lifecycle runner；隔离 bundle ID/用户偏好及恢复目录，验证机器代码段与当前 Release 一致。此隔离副本重新 ad-hoc 签名，软件行为证据与正式签名包验证分别记录。

- 浅深色文件生命周期、最近文档跨进程、SIGKILL 恢复和退出后磁盘检查共 6 个报告项通过。
- 覆盖新建/首次保存取消、Save As 冲突与失败、BOM/CRLF、自动保存、预编辑、外部修改保护、退出取消回滚、命名/未命名/原文删除、损坏记录保留和指定恢复副本丢弃。
- 正式签名 B 的浅深色窗口状态检查最终串行通过：预览/源码模式、安全归档、挂窗前解码、反向选区、侧栏、阅读锚点误差≤1pt、查找焦点、AX 正文、原文磁盘不变。
- 不将应用强制退出和 NSKeyedArchiver 解码称为操作系统注销/重启。

证据：`artifacts/group6-document-lifecycle/`、`artifacts/group6-window-restoration/`、`artifacts/group6-upgrade-a-b/`。

## 构建与验证

- `cargo fmt --all --check`、locked workspace Clippy 通过。
- workspace Rust：**1607 passed / 0 failed / 7 ignored**；ignored 不计作通过。
- Release 包资源/架构/最低版本审计通过。
- 签名前后各 19 项原生自检及 2 项资源错误路径通过；包含新增 AX 非法范围断言。
- 发行门禁单元测试：**9 项通过**。

## 失败与修正记录

- 初版新增窗口自检误直接调用继承的 TextKit `deleteBackward`，而真实按键和现有测试使用 `doCommand(by:)`。已修正测试调用。期间两次崩溃堆栈位于 MenuBarClientCore/Swift concurrency，原始日志保留，不据此声称系统缺陷已定位。
- 后续自检已完成全部空文档断言，但退出时等待新草稿保存确认而超时；仅为测试自建文档注入丢弃选择后，浅深色完整运行退出码均为 0。
- 本次 AX 设置保存面板自定义文件名后出现按钮停留；取消保留恢复正文。重新打开面板、使用原生默认名保存成功。CUA 原生粘贴也出现过超时报告，始终先读回实际状态，避免重复插入。自定义名称操作不冒充已通过。
- 首次窗口归档检查及一次深色重试遇到 current=true/presented=false 的显示等待超时；保留失败，最后单独串行两主题均通过。未修改生产呈现逻辑，不推断失败根因。
- DMG 自动容量估计不足，临时卷报告空间不足，但宿主尚余 103GiB。现在按逻辑文件大小（包含稀疏文件）加 25% 和 32MiB 余量、至少 256MiB，明确 HFS+ 后压缩成 UDZO。新增 `resume-dmg` 仅允许 app 已 Accepted、ticket/签名/包清单有效且 DMG 尚未提交时重试，保留原失败日志。新包实际创建、公证及挂载校验均通过。

## 剩余收尾

检查升级失败保全，冻结正式提交并从干净源码重建、签名和公证，复核最终 DMG 的本机安装、外部 AX 与 A→B 升级。真实 OS 注销/重启已按用户要求跳过；不为测试改系统安全设置。VoiceOver 人工朗读、默认 Gatekeeper 首启、全新用户环境及 CI 运行明确保留为未实测或后续跟踪，公开发布另行记录。

## 外部 AX 软件验收追加记录

旧实现仅合并 range 首尾光标：一个跨过长行的系统 `AXBoundsForRange` 查询返回约 35 pt 宽，漏掉中间约 600 pt 的文字。现在 Rust 在同一排版快照中合并区间内各视觉行、表格单元格和双向文本簇，再由 AppKit 转屏幕坐标。独立进程通过系统 AX 接口读到：跨行约 604 pt、跨表格单元格约 531 pt、希伯来语约 82 pt、阿拉伯语约 80 pt；空光标为 1 pt 宽，超界与溢出范围为零尺寸。测试源文件没有修改。

系统 AX 树原先无法取得编辑器的 `AXChildren`，尽管产品内部能列出语义节点。语义对象现继承 AppKit 的 `NSAccessibilityElement`，编辑器覆盖 `NSTextView.accessibilityChildren()`；外部树实际出现标题、段落、表格列分隔线及稳定 ID。语义字符串查询也在相对范围做减法校验，避免整数溢出。Rust 定向回归和原生 Accessibility 自检通过；证据在 `artifacts/group6-ax-contract/`。这份隔离应用是 ad-hoc 测试副本，正式签名包的外部复核留在最终验收。
