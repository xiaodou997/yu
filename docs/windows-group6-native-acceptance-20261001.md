# Windows 第六组 UI Automation / Contrast Theme 原生验收

日期：2026-10-01（Asia/Shanghai）。代码基线：`main@9eac191` 加本轮实现与门禁修复。
环境：Windows 11 专业版 x64，10.0.26100 / 24H2，本机单显示器；实际测试窗口
192 DPI（200%），工作目录所在卷为 NTFS。

结论：**第六组软件实现和本机自动验收完成。** 已从独立进程通过 Windows UIA
访问真实编辑窗口，验证正文、语义节点、动作、事件和生命周期；高对比度绘制通过实际
D3D 目标像素读回验证。Narrator 实际语音和系统对比主题实际切换未执行，不计入通过声明。

## 实现与产品行为

- 编辑 HWND 接入 `WM_GETOBJECT`，暴露 Document、TextPattern / TextPattern2、
  单选区、caret、可见范围、屏幕矩形、查找、文本范围移动及 ScrollIntoView。
- 文本和 UTF-16 坐标来自共享 `TextSnapshot` / `AccessibilityTextSnapshot`。
  字符移动使用扩展 grapheme，保持 emoji、ZWJ、组合字符与 CRLF 完整；`GetText`
  的 UTF-16 长度上限不会截断代理对。Unicode 大小写搜索映射回原始 source offsets。
- 标题级别、任务复选状态、链接和图片名称来自共享 Markdown 语义快照。任务名称
  去掉 parser 已识别的列表 / 复选框标记，状态单独通过 TogglePattern 暴露。
  HTML 标签和实体复用已有 HTML model 的可访问文本。
- Select、ScrollIntoView、Toggle 经所属 HWND 执行，复用原有选区、滚动和 editor
  transaction；任务动作可撤销。活动 IME composition 时拒绝替换其 canonical range。
- COM provider 持有不可变快照和共享请求队列；不保存 AppWindow 裸指针。
  mutation 请求有超时，校验文档身份和 revision；旧语义节点 / 文本范围及关闭后的
  provider 返回不可用。窗口销毁时断开 provider 并清理控件名称注解。
- 文本、选区、语义树、键盘焦点与几何变化向外部 UIA 客户端发出通知。
  隐藏窗口不返回可见文本范围，重新显示恢复几何；移动 / resize 更新实际 screen coordinates。
- F6 / Shift+F6 在正文与可见侧栏控件之间切换焦点；正文的 Tab 输入保持原有语义。
  标准 Win32 搜索框通过 `Name_Property_GUID` 设置可访问名称。
- 读取 `SPI_GETHIGHCONTRAST` 和系统 Window / WindowText / Highlight / HighlightText
  颜色，覆盖 chrome 与共享 scene 的对应角色；对比模式禁用 Mica。
  不透明选区先绘制，随后绘制选中文字，避免遮住文字。普通模式使用原有配色。
- 对比配色进入 frame build key；主题变化隔离旧 frame / resource publication。
  embedded helper 的样式上下文同时接收前景色，不改 Markdown 内容。

UIA 接入参考 [Microsoft provider 返回接口说明](https://learn.microsoft.com/en-us/windows/win32/api/uiautomationcoreapi/nf-uiautomationcoreapi-uiareturnrawelementprovider)
和 [Text / TextRange 实现约定](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-implementingtextandtextrange)。
控件名称注解按 [Microsoft Win32 Edit 名称示例](https://learn.microsoft.com/en-us/accessibility-tools-docs/items/win32/edit_name)
处理；系统颜色读取参考 [High Contrast 参数](https://learn.microsoft.com/en-us/windows/win32/winauto/high-contrast-parameter)。

## 已执行验证

| 验证 | 结果 |
| --- | --- |
| 独立 UIA 客户端 | 37 项通过；原生 CUIAutomation 检查标准控件，System.Windows.Automation 检查自定义正文 provider |
| 正文与 Unicode | 完整 canonical Markdown 一致；无匹配返回 null；大小写查找、代理对长度限制、选区、可见范围、克隆与跨 COM 比较通过 |
| 语义与动作 | 标题、RangeFromChild、任务名称 / 状态 / Toggle、Undo、旧 revision 范围失效通过 |
| 焦点与几何 | SetFocus、F6 / Shift+F6、实际 move / resize、隐藏 / 显示状态、visible ranges 恢复通过 |
| 外部事件 | 已收到 selection、text、structure、focus、layout 通知；fixture 进程退出后对象不可用 |
| 高对比度实际 GPU 像素 | 原生窗口使用注入的黑底 / 黄字 / 蓝选区 / 白选中文字；实际 D3D 读回分别检测到对应像素；恢复普通配置通过 |
| 原生 Windows 测试 | font 20 / 20，render 4 / 4，shell 22 / 22；另 1 项默认 ignored 的第五组资源集成由 self-check 显式执行通过 |
| HWND / DirectWrite / D3D smoke | helper / GUI 构建、check、真实窗口 Present 与退出通过 |
| 完整 workspace 回归 | 1,532 passed、0 failed、4 默认 ignored；其中资源集成另行执行通过，其余是生成表格专门验收、诊断报告、随机 fuzz soak |
| 共享模块专项回归 | editor / scene / workspace / assets / render / storage 共 795 passed、0 failed、1 既有 ignored |
| 严格检查 | `cargo clippy --workspace --all-targets -- -D warnings`、fmt、diff check 通过 |
| 依赖方向 | 按既有 `tools/check-deps.py` policy 用 PowerShell 核对 cargo metadata：27 包 / 114 条内部边通过；本机未运行 Python 解释器 |

UIA fixture 使用独立 UTF-8 副本。读取、导航、任务 Toggle + Undo、resize 后文件哈希
保持一致；测试仅关闭自己新建的 fixture 进程。没有更改系统缩放、对比主题或 Narrator
设置，没有关闭已有用户文档或第五组预览窗口。

PowerShell 5 的旧托管客户端会影响本进程的标准控件 proxy factory；测试先以原生
CUIAutomation 检查 Edit / Button 的控件类型和名称，再用托管客户端访问 Yu 的
自定义 provider。这一顺序保留了实际系统 UIA 访问路径，无需改写标准控件 provider。

## 全仓门禁发现并修复的问题

| 问题 | 修复与验证 |
| --- | --- |
| macOS 专用字体分支在 Windows 上触发 dead-code；Metal 非 macOS 回退误引用不在作用域的参数 | 对专用分支补齐 cfg，纠正回退参数；macOS 原有分支保留，全 workspace Windows 编译 / clippy 通过 |
| FFI 通用测试缺少 ByteOffset 导入；macOS 专用 helper 在 Windows 产物中未使用 | 调整 test / target cfg；Windows 全仓 FFI 测试通过 |
| Windows PNG 分段导出创建临时目录后，被父目录时间戳变化误判为目录替换 | 复用已锁定的 `same-file 1.0.6` 保持父目录 OS 身份 handle；正常分段发布和真实父目录替换拒绝回归通过 |
| Windows 同长度快速图片改写可能保留相同时间戳，漏过 frozen resource 复核 | 在 metadata 检查之外用 64 KiB 缓冲逐段比较冻结内容；原有失败用例和显式恢复 modified time 的新回归通过 |

目录与导出回归在本机 NTFS 执行；ReFS、网络文件系统和 macOS 原生运行不在本轮验证范围。

## 复现与本机证据

仓库入口：

```powershell
./platform/windows/yu-shell-windows/run-self-checks.ps1
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

单独执行外部客户端（先构建 GUI 和 companion helper）：

```powershell
cargo build -p yu-shell-windows -p yu-document-renderer
powershell.exe -NoProfile -ExecutionPolicy Bypass -Mta -File platform/windows/yu-shell-windows/verify-accessibility.ps1
```

Fixture：`platform/windows/yu-shell-windows/Fixtures/group6-accessibility.md`。
本机 ignored 证据包括 `artifacts/windows-group6-self-checks.log`、
`windows-group6-workspace-full.log`、`windows-group6-shared-full.log`、
`windows-group6-clippy.log`、`artifacts/windows-group6/client/results.json` 及
`artifacts/windows-group6/20261001/` 的 metadata / dependency audit。
JSON 记录实际 executable SHA256、fixture SHA256、原生 DPI 和各类事件次数；不提交本机产物。

## 验收范围与后续

- Narrator 实际朗读、扫描导航及声音反馈未运行；UIA 互通通过不等于语音体验已验收。
- 系统 Contrast Theme 的真实启用 / 切换 / 关闭未执行。通过的是原生颜色读取接线、
  注入配色的真实 GPU 像素及配置恢复；全部系统主题 / 自定义配色兼容性未声明通过。
- 当前文本模式读取 canonical Markdown。没有实现 UIA TextEdit 的独立 preedit 事件，
  不把 IME overlay 写进源文；复杂朗读、公式专用语义及链接 Invoke 仍不在本次声明内。
- TextUnit Character / Word / Paragraph / Document 已支持。Line / Format 提升为
  Paragraph，Page 提升为 Document；本次没有宣称完整视觉折行单位导航。
- 实际跨 DPI / 多显示器验收继续保持未执行；用户之前跳过的缩放测试没有补记为通过。
- 下一组为 Windows 第七组：MSIX、Store metadata、exe icon / resource、签名和发布包装。
  第六组的人工体验验收仍可另行补充，不以打包完成替代。
