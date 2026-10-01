# Windows 第五组原生实现与自动验收

日期：2026-10-01（Asia/Shanghai）。代码基线：`main@84a3924` 加本轮第五组实现。
环境：Windows 11 专业版 x64、Intel Iris Xe、本机单显示器，实际 Yu 窗口 192 DPI（200%）。
用户已授权进入下一阶段并由代理自行验证；不再安排本轮已跳过的系统缩放操作。

结论：本轮图片、公式、Mermaid、GPU 资源与 COLR 彩色 emoji 产品链已实现，
原生自动验收及可见窗口检查通过。以下限定范围内的结果可以复现；完整格式兼容性、
真实驱动故障与全部人工输入矩阵不在该结论内。

## 产品行为

- 本地 PNG 等 WIC 图片和透明 SVG 在后台解码，保留原始 intrinsic 尺寸；缩略像素
  不改变布局事实。相对路径以当前文档定位，Unicode 文件路径保留。
- 行内 / 独立公式及 Mermaid 复用 `yu-document-renderer.exe` 和共享协议，输出 SVG
  后进入同一 D3D image texture 路径；公式 baseline、主题和实际 raster scale 随 publication 传递。
- 资源完成后更新共享布局与可见帧，不改源文、revision 或 dirty；错误资源显示原始源文，
  不持续重复提交同一失败任务。没有发布结果时继续显示共享 fallback。
- 后台线程独立初始化 COM；单工作线程最多 4 个在途请求。revision、文档身份、路径、
  主题和 DPI 更新隔离旧结果；前台 50ms timer 仅在收到结果时触发资源重绘。
- 图片 CPU 缓存 32 项 / 64MiB；embedded CPU pixels 16 项 / 64MiB；SVG markup、
  输入文件与解码尺寸有上限。GPU 仅保留当前所需的已发布资源，恢复后重新上传。
- DirectWrite COLR v0 palette layer 栅格化为共享 premultiplied RGBA glyph bitmap，
  普通 glyph 继续原有 alpha 路径。本机 `😀`、`👩‍💻`、`👍🏽` 验证了实际彩色像素和 2x 尺寸。
- Windows helper 使用带 pipes 的隐藏进程启动方式，加载公式 / Mermaid 不弹辅助控制台。

新增本机 fixture：`platform/windows/yu-shell-windows/Fixtures/group5-resources.md`
及 `group5-errors.md`。正常 fixture 包含本地 PNG、透明 SVG、CJK / emoji、行内公式、
独立公式和带中文标签的 Mermaid；错误 fixture 包含缺失图片及无效嵌入源文。

## 本轮发现并修复的问题

| 问题 | 修复与验证 |
| --- | --- |
| 普通图片也提前计算 `kind - 1`，debug 下减法下溢 | 先筛选 embedded 类型再计算；正常图片真实 D3D 渲染通过 |
| 重建 flip swapchain 时旧对象仍占用 HWND，返回 E_ACCESSDENIED | ClearState、释放 views / swapchain / textures、Flush 后重建；原生重建与像素一致性通过 |
| 已编辑文档换成新文档后 revision 从初始值重新开始，被旧 frame gate 判过期 | 单独记录文档身份并重置共享 publisher / GPU consumer；保存后新身份重开仍能渲染资源 |
| CPU 缩略图像素被误当原图布局尺寸，加载或逐出可能跳动 | ready 与 metadata 均使用 intrinsic 尺寸；20×10 像素、800×400 intrinsic 回归通过 |
| 资源失败转为 source 后下帧不再发现 widget，丢失失败标记并重新请求 | revision 内保留失败源范围；缺失 helper 和非法源文反复重绘不增加任务 |
| DPI 路径构造新 renderer 时旧 flip swapchain 尚存在 | 复用当前 renderer resize，重建 shaper / builder / resource context；当前 DPI 原生回归通过，实际系统切换本轮未执行 |

GPU 验收按需在 Present **之前**读回 render target，避免 flip-discard 的提交后内容失效。
生产窗口默认没有读回开销。重建验证使用主动重建 D3D 对象，不伪称已触发真实驱动 reset。
swapchain 释放顺序参考 [Microsoft D3D11 Flush 文档](https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11devicecontext-flush)。
COLR layer 获取参考 [Microsoft TranslateColorGlyphRun 文档](https://learn.microsoft.com/en-us/windows/win32/api/dwrite_2/nf-dwrite_2-idwritefactory2-translatecolorglyphrun)。

## 已执行门禁与证据

| 验证 | 结果 |
| --- | --- |
| `yu-font-windows` 原生测试 | 20 / 20，通过 COLR 彩色样本、RTL、字体 fallback 与实际 glyph rasterization |
| `yu-render-windows` 原生测试 | 4 / 4，含透明 SVG pixels、外部 SVG image resolver 禁用和 thumbnail 约束 |
| `yu-shell-windows` 常规测试 | 21 / 21；另 1 项资源集成默认 ignored，由 self-check 显式执行通过 |
| 真实资源集成 | 2 张图片 + 2 个公式 + 1 个 Mermaid，source / revision / dirty 不变；actual target 彩色像素存在；重建后像素完全一致 |
| 原生输入模型集成 | 20 次 composition commit + 20 次 cancel，每轮原生重绘与 Undo/Redo；preview / cancel 不写 canonical source |
| Unicode 保存与重新打开 | `保存 重开 😀.md`，严格 UTF-8 读取逐字节一致；新文档身份及资源链正常 |
| 深色与失败回退 | 深色 math / Mermaid publication 正常；缺失图片、非法公式 / Mermaid 共 3 个失败稳定回退，5 次重绘不重复请求 |
| 异步边界 | 旧 revision 回包不能发布进新文档；缺失 helper 稳定回退，source 不变 |
| 持续原生渲染 | 600.01 秒、2,894 帧；滚动与资源呈现完成，无测试异常；完整测试 604.88 秒 |
| 共享回归 | `yu-workspace` / `yu-assets` / `yu-render` / `yu-storage` / 完整 `yu-editor` 合计 777 passed、0 failed、1 既有 ignored |
| `yu-embedded-client` | 2 / 2，通过现有 transport 测试 |
| 原生 build / check / HWND smoke | helper 与 shell 构建成功，self-check 完整退出 0 |
| `clippy --all-targets -- -D warnings` | shell / font / render / embedded-client / workspace 通过 |
| `fmt --check` / `git diff --check` | 通过 |
| 依赖方向 | PowerShell 读取既有 `tools/check-deps.py` policy 和 cargo metadata，核对 27 个包 / 114 条内部边通过；本机无 Python，因此不是执行该 Python 脚本的记录 |

持续验证期间抽样观察 working set 约 92.7→92.9MiB、private bytes 约 79.9→80.1MiB，
handles 316→312–314；抽样只描述该次运行，不是长期无泄漏证明。

可见窗口检查了真实屏幕上的图片、SVG 中文、两种公式、中文 Mermaid 与彩色 emoji。
独立预览进程 PID 19220，文件哈希保持不变，资源诊断稳定为 2 images / 3 embedded /
0 in-flight / 0 failures / 0 stale。截图 `preview-ready.png`；另以定向 WM_MOUSEWHEEL
滚动采集 `preview-mermaid.png`，确认 Mermaid 完整节点与连线，随后恢复原滚动位置。

本机忽略目录：`artifacts/windows-group5/20261001/`，保留 `manifest.json`、
`self-checks.log`、`resource-soak.log`、`shared-tests.log`、`clippy.log`、
`dependencies.log`、截图、输入副本、Unicode 保存结果和独立二进制。
这些本机产物不提交 Git；fixture、代码及复现入口提交。

| 产物 | SHA256 |
| --- | --- |
| shell，PE32+ x86-64 GUI | `E5F89FFFF52A8FB70E818952A687EC4AC22D019841F24122BF3A206194F1A0C7` |
| companion helper，PE32+ x86-64 | `A32B0B90817F0A5C7B38B6B2519195AD819901D06BEE27B76F9B3F26DFA9E557` |
| 预览 Markdown | `1B3A4B840F259852870CB408C7E19858BF5F2B75C6D8D133A563F2DFF5C9EB97` |

## 复现

在 Windows x64 的仓库根目录运行：

```powershell
./platform/windows/yu-shell-windows/run-self-checks.ps1
```

该脚本先构建 companion，再运行原生单元测试、显式资源集成、check 和窗口 smoke。
已由既有 Windows CI job 调用，不需要手动增加另一个入口。

额外执行 10 分钟集成 soak：

```powershell
$env:YU_GROUP5_SOAK_SECONDS = '600'
try {
    cargo test -p yu-shell-windows native_group5_resources_present_recover_and_preserve_source -- --ignored --nocapture
} finally {
    Remove-Item Env:YU_GROUP5_SOAK_SECONDS -ErrorAction SilentlyContinue
}
```

运行可见资源 fixture：

```powershell
cargo build -p yu-shell-windows -p yu-document-renderer
./target/debug/yu-shell-windows.exe ./platform/windows/yu-shell-windows/Fixtures/group5-resources.md
```

分发时两个 exe 必须位于同一目录；helper 缺失时图片仍可加载，公式 / Mermaid 回退源文。

## 保留的范围与下一阶段

- 本轮原生实际样本为 PNG / SVG；WIC 其他图片格式、动画、EXIF orientation 和完整
  色彩管理矩阵没有逐项验收。远程 / data 图片继续遵循共享路径解析策略，未新增网络加载。
- SVG image href / data resolver 禁用，不隐式读取额外文件或网络；外部图片引用的 SVG
  不属于本轮展示兼容性范围。resvg 使用 `0.48.1` 的 text / system-fonts 支持。
- 彩色路径验证 COLR v0 与本机 Segoe UI Emoji；PNG / SVG / COLR v1 彩色字体、
  国旗字体呈现和所有 emoji 序列未逐项覆盖。
- 实际系统缩放切换按用户要求未执行；跨显示器不同 DPI 因单屏环境未执行。
  程序内 glyph 2x 回归不能代替这两项。
- 模型 composition、保存重开与持续渲染补测不代替真实 IME 连续操作、Yu↔记事本物理
  剪贴板或文件对话框 Save / Discard / Cancel 全分支人工验收。第四组原始矩阵例外继续保留。
- 主动 renderer 重建已验证，真实 GPU driver reset / removal 和长期资源压力兼容性待后续运行收集。

下一阶段是第六组 UI Automation / Narrator / Contrast Theme；MSIX / 签名与发布为第七组。
