# macOS 自动窗口回归与性能采样

## 运行入口

在正常 macOS 图形会话中运行，使用 Xcode 与本仓库的 debug app：

```sh
./platform/macos/yu-shell-macos/build-app.sh
./platform/macos/yu-shell-macos/run-render-regression.py .notes/render-regression-new
```

输出目录必须不存在，避免覆盖历史证据。`--case long` 或 `--case resources`
只重跑对应场景；`--prepare-only` 只生成测试文档与图片，不启动 app。
每次记录二进制/fixture SHA-256、Git 状态、硬件、macOS、Xcode、窗口尺寸、
scale、屏幕最大刷新率、原始日志和各阶段 p50/p95/p99。

### 自动化覆盖

- 324,245 字节长文档，含 350 个章节、中英/RTL/组合字符、表格和引用。
- 首帧在顶部，12 次分段滚动、4 次窗口缩放，内容高度与滚动范围一致。
- 导航到末行：同时检查 caret 查询与已提交 Metal 帧中的 caret，不能只看
  查询返回 `needsScroll=false`。
- detach/rebind；实际关闭 NSWindow 后，用新 session/window 重开并验证顶部首帧。
- 图片延迟 14 秒、Math 延迟 16 秒，同时保留 Mermaid `Unsupported`。
- pending 时 coverage 内滚动保持 frame serial，使用 retained presentation。
- 停止操作后只读取 coordinator 已提交的 snapshot，不提交帧、不查询资源，
  验证完成通知能主动更新窗口；图片高度只变一次，完成后不再空转发布。
- 正常 pending 不得触发 `resource_retry` 定时重试。

延迟只在 Rust `debug_assertions` 构建启用：`YU_TEST_IMAGE_DELAY_MS` 与
`YU_TEST_MATH_DELAY_MS` 接受 1–30,000ms，release 构建不包含注入逻辑。
测试不模拟 live-scroll 起止事件，所以不会把脚本驱动的帧间隔计入真实触控板验收。
资源测试验证 Math pending/通知状态，不证明 Math 内容在画面中的最终视觉质量。

## Instruments

使用生成的 fixture 录制相同的自动场景：

```sh
./platform/macos/yu-shell-macos/record-instruments.sh \
  .notes/render-regression-new/fixtures/long.md \
  .notes/instruments-long-new 60s --render-regression

./platform/macos/yu-shell-macos/record-instruments.sh \
  .notes/render-regression-new/fixtures/resources.md \
  .notes/instruments-resources-new 60s --resource-regression
```

自动场景失败时保留 trace、日志和统计结果，并以非零状态退出。导出 CPU 调用栈：

```sh
xcrun xctrace export --input .notes/instruments-long-new/render.trace \
  --xpath '/trace-toc/run[@number="1"]/data/table[@schema="time-profile"]' \
  --output .notes/instruments-long-new/time-profile.xml
```

不传自检模式时是实际交互采样，需要用户在打开的窗口中连续触控板滚动。
应先约好采样时间，再开始录制；尚未完成的手势和没有样本不能被判为达标。

## 2026-09-11 实测

硬件 Apple M1 Max（Mac13,1），macOS 26.5 / 25F71，Xcode 26.6 / 17F113。
debug 构建，2x，屏幕报告最大 60Hz。首次窗口 900×692pt；重开窗口 900×652pt。

| 场景 | 结果 | 证据 |
| --- | --- | --- |
| 延迟图片、Math、Mermaid 状态 | 通过 | 2 次完成通知，0 次资源定时重试；pending 滚动保留 publication；闲置后完成更新，高度改变 1 次 |
| 长文档顶部、滚动范围、12 次定位、4 次 resize | 通过对应断言 | 每次均提交非空当前 Revision 帧，extent 与 publication 相符 |
| 长文档末行导航 | **失败** | 查询认为 caret 已可见，但实际帧 caret 数量为 0 |
| detach/rebind 与实际关闭/重开 | 通过对应断言 | 新 publication serial；重开回到顶部且提交首帧 |
| 连续触控板 p95 ≤ 16.7ms | **未验收** | 本轮仅脚本输入，无 live-scroll 手势样本 |

原始证据：

- `.notes/macos-render-regression-20260911-a/`：资源通过、长文档首次失败。
- `.notes/macos-render-regression-20260911-b/`：按正确正文宽度重测末行，仍失败；
  独立完成关闭/重开检查。`long-summary.json` 保持 `passed=false`。
- `.notes/macos-instruments-long-20260911-a/`：真实 Metal System Trace、CPU
  time-profile 导出和统计，目标应用因末行断言退出；xctrace 返回 54，不能当作通过。

末行复现值：caret 查询 Y=28,001.8125pt，当前 scrollY=27,415.5pt，
viewport=624pt，`needsScroll=false`；已提交帧 `caretDecorationCount=0`，
内容高度=31,393.6875pt。代码中 caret 查询使用 canonical editor 的增量高度状态，
而 `publish_owned` 重建独立 EditorDocument，输入只传 ViewportConfig，没有传递
测量后的高度状态。这是定位与渲染不一致的明确修复方向，仍需实现与回归验证。

非 Instruments 自动长文档采样：后台 preparation p50=615.55ms、p95=666.47ms；
Swift submit attempt p99=561.99ms；Metal encode/submit p95=1.23ms。
这些是离散脚本步骤的阶段耗时，不是连续呈现帧时间。

Instruments CPU 采样中，主线程共约 16.271s 样本权重，
`DocumentTextView.refreshTableResizeAccessibility` 的包含子调用权重约 4.887s；
下游 `yu_storage_session_table_resize_accessibility_dividers` 约 4.693s，
并调用 `visible_blocks_with_shaper`。这些数值相互包含，不能相加；它们说明
主线程仍有辅助功能几何查询触发的排版工作，不能仅凭 Metal 编码较快宣称流畅。

下一步应先统一长文档定位与 publication 的布局状态，再分析并减少这条主线程
重复布局路径。真实 IME、VoiceOver 操作、跨显示器、残影与触控板体验仍需人工配合。

新增入口后的基线检查：app 构建成功，原有 Dark Aqua 真实窗口 self-check 通过；
`yu-render-macos` 10 项通过、2 项原有 ignored，metrics 统计测试 3 项通过。
这里的通过不覆盖上表明确失败的长文档末行用例。
