# Windows 第四组原生阻塞修复记录：2026-10-01

**当前结论：原生软件门禁已通过，完整 IME 人工验收仍待完成，第四组尚未正式结项。**

测试基线是 `main@83e1bdb27740f8cc623622117316fd596332702a` 加本轮原生修复。
日期按 Asia/Shanghai。当前机器为 Windows 11 x64、本机键鼠、单显示器。
读取到当前 Yu 窗口 DPI 为 **192（200%）**；本轮没有改变系统缩放设置。

## 修复内容与根因

### W-PRE-02：TSF 初始化将成功误报为失败

启动阶段诊断将问题定位至 TSF 初始化。
`ITfThreadMgr::AssociateFocus` 在没有旧关联时返回 S_OK，同时将旧 document manager 设为 NULL。
仓库使用的 windows-rs 0.58 生成包装要求返回非空 `ITfDocumentMgr`，
将这个合法空值转换成 `Error(S_OK)`，导致窗口初始化退出 1。

修复在 Windows TSF adapter 中按原始 COM 接口接收可空输出，检查真实 HRESULT，
正确接管非空旧接口的引用，并在销毁时尝试恢复旧关联。真实 HRESULT 失败仍继续传播。
新增真实 HWND/TSF 回归覆盖首次空关联、替换旧关联和解除关联。

接口依据：[Microsoft AssociateFocus 文档](https://learn.microsoft.com/en-us/windows/win32/api/msctf/nf-msctf-itfthreadmgr-associatefocus)。

### W-PRE-01：RTL cluster 翻译误用文本方向

真实 DirectWrite RTL shaping 返回按逻辑文本顺序索引的 clusterMap；
原代码按 RTL 方向要求它递减，导致 Hebrew 在文本单元 1 被拒绝。
现先按数组的逻辑索引建立完整 source cluster，再把 RTL glyph 数组转换成共享布局
从左到右的绘制顺序。DirectWrite 的 advanceOffset 沿文本方向定义，RTL 时转为相反符号的 x offset；
y offset、advance、glyph id 与 source cluster 随 glyph 一起保留。

回归覆盖 RTL glyph/source 对应、多 glyph 组合簇、零 advance、非零 offset 的实际位置。
真实 DirectWrite 测试增加 Hebrew/Arabic 组合符及 ASCII/RTL/emoji 混合文本。
没有通过关闭单调性检查、吞掉错误或丢弃字形使测试通过。

offset 依据：[Microsoft DWRITE_GLYPH_OFFSET 文档](https://learn.microsoft.com/en-us/windows/win32/api/dwrite/ns-dwrite-dwrite_glyph_offset)。

### W-PRE-03：可见正文挤叠在左上角

窗口能启动后，实际 200% DPI 截图显示 Unicode 样本正文严重重叠。
CPU draw-command 诊断确认这些坐标已经在提交 GPU 前出错；并非仅有 DPI 截图坐标问题。
Windows 壳设置了 render viewport，却未给共享编辑器设置 viewport layout。
共享编辑器因此仍使用默认 `LayoutConfig(80, 1)` 和估算块高 1；
实际 16 DIP 字体被压进极窄折行与极小行距。

现每次发布渲染快照前，用实际正文逻辑宽度和字号对应的行距同步共享 editor viewport，
只在配置改变时更新。窗口 resize/DPI 更新后，下一帧的换行、绘制、hit-test 与 TSF 几何
均从这份配置生成。配置变更不修改 canonical source 或 revision。
可见截图已确认当前 200% DPI 下正文恢复可读。

### W-PRE-04：选区和 caret 未接入 scene

同轮检查发现 Windows render config 没有启用共享 editor decorations，
没有 Win32 caret 作为替代。现使用共享 Appearance 主题的 selection/caret/composition-caret 样式。
新增 native render 回归通过实际帧断言 caret 和 selection primitive 的存在；
没有引入独立的平台正文/选区绘制路径。

W-PRE-03/04 的回归创建真实 surface/D3D renderer，
验证 readable line boxes、resize 后折行减少、布局宽度跟随 surface、
caret/selection 像素由共享 scene 持有，以及 viewport 同步不推进 revision。

## 最终验证

| 项目 | 结果 |
| --- | --- |
| yu-font-windows native tests | 19/19 PASS |
| yu-render-windows tests | 2/2 PASS |
| yu-shell-windows native tests | 16/16 PASS |
| 原生 cargo check 与 exe 链接 | PASS |
| 真实 HWND / DirectWrite / D3D first Present / TSF / clean close self-check | PASS，退出 0 |
| yu-editor 回归 | 573 passed、0 failed、1 ignored，包含 doc-test |
| 三个 Windows crate 的 all-targets clippy -D warnings | PASS |
| cargo fmt --all -- --check | PASS |
| git diff --check | PASS |
| 当前 200% DPI 下 Unicode 样本可见呈现 | 已实际检查正文可读；不是全部交互通过记录 |
| 真实 IME、键鼠完整人工清单 | NOT_RUN / 等待操作结果 |
| 单屏 100% / 125% / 150% 切换 | NOT_RUN |
| 跨屏 DPI | BLOCKED，只有一块显示器 |

Python dependency-check 脚本本轮未重跑：未找到可用解释器，桌面工具也未配置 bundled runtime。
本轮没有更改任何 Cargo 依赖；没有把该检查记为 PASS。

测试 exe：`target/debug/yu-shell-windows.exe`。
SHA256：`084E90C96AF5402413D1AD4701E14084F6B201ED00EA449C00CF9D2CA4825076`。

本机证据目录（Git 忽略）：`artifacts/windows-group4/20261001-fixed/`。
主要文件：`native-self-check.log`、`editor-regression.log`、`clippy.log`、
`unicode-window.png`（初次可见重叠）、`unicode-window-fixed.png`、
`window-geometry.json`、`render-diagnostic.log` 与 `render-fixed-diagnostic.log`。
临时 render trace 已从产品源码移除。

## 下一步

用户随后提供的窗口截图暴露出未修复的产品壳缺口：侧栏只是“文件 / 大纲 / 搜索”
占位文字，侧栏与状态栏没有配置 DPI 对应的 UI 字体，也缺少导航样式与内容面板。
这些项目随后已完成基础修复与当前 200% DPI 的可见窗口检查，
详见 [侧栏与滚动修复记录](windows-chrome-native-fix-20261001.md)。
完整物理键鼠、真实 IME 与其他缩放比例仍待人工验收。
上述自动化通过结果只证明对应测试覆盖的原生链路，不能解释为完整界面已完成。

已启动当前修复版本的 `unicode-work.md — Yu` 窗口，供本机操作。
先在第一行或任意可见正文位置用微软拼音验证候选/提交/取消，再验证 Undo/Redo。
记录实际结果后按 [完整人工验收计划](windows-group4-manual-acceptance.md) 扩展至五笔、日文、
韩文、键鼠和 DPI。自动初始化通过不能代替真实 composition/candidate 操作。
