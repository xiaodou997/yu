# 第四组收尾第五轮：本机重试验证与真实外观门禁

更新：2026-09-26。接手基线 `4fc2a99151e890a38037360c278ea237016d8d2f`。本轮使用用户授权的 WebCodex Local，在 `yu-workspace` 的 macOS 27／arm64／Xcode 27 上实际构建和验证。不是 macOS 26 实机结论；第四组仍未结项。

## 1. 接手与范围

本地与远端 main 一致。继承的三个已修改状态文档、第二／三／四轮未跟踪文档均已备份到 `artifacts/group4-local-20260926-05/inherited/`，本轮不覆盖或提交这些草稿。无关的 `MindLoci-auth-check/` 未读取、修改、删除或提交。不能将本轮文件已提交说成整个工作区干净。

保留 `f3762f87` 的末帧丢弃恢复和 `4fc2a991` 的重试预算修复。本轮未修改 Metal、着色器、缓存容量、系统时钟、CI、导出或旧 RC3 标签。

## 2. 上一批重试修复已在 Mac 验证

八项 `embedded_retry_lifecycle` 集成测试实际全部通过。相关 `yu-assets`／`yu-embedded-client`／`yu-storage-ffi` 共136项测试通过、0失败、0忽略，包含这八项，不重复累加。Clippy 全target `-D warnings` 通过。

首次全仓格式检查发现新增测试需要 rustfmt 排版。只格式化该测试文件后，相关136项重新通过，全仓 `cargo fmt --all --check` 通过，未改变重试语义。初始失败记录保留在本轮 `format.log`；`related-tests.log` 和 WebCodex 执行记录保留实际结果。

Homebrew Python 3.14 未安装 Pillow，首次工具测试因导入失败中止；改用已有 Pillow 11.3.0 的 `/usr/bin/python3`，未安装包或修改系统环境。旧55项工具回归通过；加入本轮外观回归后最终60项通过，见 `python-final.log`。这些是工具测试，不是60次实窗验收。

## 3. 发现并修复外观误标

初次以应用 SHA256 `68e0e20836a0c27f0fe1931090766fe2396778cbfa591a0e1041b363f69ecb6e` 运行 `cold-light`，原脚本返回成功，固定公式区域有119个对比像素。但实际查看 `restored-preview-0-197281.png`，窗口和正文是深色：没有 `--dark` 只是跟随系统，不等于显式浅色。该次可证明公式可见，**不能算浅色验收**。原始结果与截图不修改；不由此推断所有历史浅色记录均无效。

最小修复：

- 宿主增加与既有 `--dark-mode` 对称的 `--light-mode`，覆盖本进程的文档／设置窗口外观；未指定时仍跟随系统。相互冲突的显式开关在打开文档前拒绝。
- 两个资源测试入口显式传入浅／深色，并通过进程参数固定 Yu 正文主题，避免持久主题覆盖测试条件；不修改系统外观或用户持久偏好。
- 既有 `--themes` 专项不固定正文主题参数，保留原生设置中实际切换 Night／Github／Yu 的能力。
- 冷重开像素门禁同时记录背景亮度和请求／实际外观。外观不符或无法分类即失败，即使公式有像素也不能通过。原公式对比阈值、精确源码、历史及禁止缩放／滚动恢复的要求不变。

新增五项工具回归覆盖参数、主题切换例外、模糊背景拒绝、有公式但外观错误拒绝，以及正确外观只读观察通过。

## 4. 当前产品构建与冷重开

| 项目 | 实际身份／结果 |
| --- | --- |
| 主程序 SHA256 | `548f6903480512136e3d0c34eb185ad47840670a6c7824be8a0d2d8c30bf8330` |
| helper SHA256 | `f74c4f402bf005eee852db83f272de35c512d43bffb5e868c84da32ca769f68e`，未变化 |
| shader SHA256 | `b5f1563b90031cdf588fd00166136fbdcda9dba94df5751324142f46c7b9bbb7`，未变化 |
| Release 构建和产物审计 | 通过，`build-appearance.log`、`build-appearance-manifest.json` |
| 当前构建原生自检 | 19项通过；另有Swift日历、调度器、拼写及图片契约检查，`native-appearance.log` |
| 桌面预检 | 解锁、辅助功能、事件投递与屏幕录制全部通过，`preflight.json` |
| `cold-fixed-light` | 完整退出／新PID重开、源码／历史／保存通过；背景255，公式182个对比像素 |
| `cold-fixed-dark` | 同链路通过；背景约32.67，公式119个对比像素 |

实际复核了上述两次重开的完整窗口截图，公式与外观均正确。像素门禁仅检查固定 x² 区域，不证明任意数学内容正确，也不代表所有冷重开场景均已穷尽。测试隔离包重新签名后的哈希在各自结果中单列。最终产品只改显式外观开关；后续测试参数分支调整未重新构建或改变产品二进制。

## 5. 首次真实同进程多文档链路

`soak-short-light` 的2个工作文档加1个纯文本保活窗口已真实运行，退出码0。请求压力30秒，完成3个全遍历轮次，实际循环31.467秒；两段70秒空闲、逐文档历史验证、正常关闭、同PID重开资源和新PID冷重开均完成，总计205.406秒。不是30秒整套总时长，也不是长时验收。

同进程为91330，冷重开为92133。固定公式182个对比像素，实测浅色；正常结束后本轮隔离应用和helper均无残留。原始机器记录见本地 `soak-short-light/result.json`、`events.jsonl` 和 `app.log`。

应用物理足迹：保留文档和原历史的空闲阶段从147,768,424降至141,755,496字节；关闭两个工作文档后为67,126,352字节，随后空闲末为67,028,048字节；新进程固定公式样本为62,784,520字节。关闭阶段的下降说明本次样本中有随文档关闭而释放的驻留资源，**不能分解成编辑历史／GPU各占多少，也不能作为无泄漏证明**。初始纯文本保活窗口样本为61,129,736字节，不把关闭后的差值武断归因于泄漏。

工具始终记录 `group4_signoff=false`、`memory_verdict=not_determined`。普通文档的嵌入纹理、逻辑RGBA字节和过期失败记录要求为0；未通过清空历史或缓存取得下降。

## 6. 接续范围

四文档600秒压力正在本轮独立入口执行，未将进行中的任务预记为通过。结束结果另行补充。原单文档600秒回归、更多主题／更长时多文档、分配栈归因、真实跨午夜／多日睡眠／时区事件、76份表格与96份列表完整选区排列及长页全图仍按实际缺口保留。

复现当前短链路，先从 main 重新构建并审计 Release，再使用不存在的输出目录：

```sh
/usr/bin/python3 -m unittest discover -s tools -p 'test_group4_*.py' -v
/usr/bin/python3 platform/macos/yu-shell-macos/run-embedded-checks.py --cold-resource-reopen --resource-audit --reopen artifacts/group4-next/cold-light
/usr/bin/python3 platform/macos/yu-shell-macos/run-embedded-checks.py --cold-resource-reopen --resource-audit --reopen --dark artifacts/group4-next/cold-dark
/usr/bin/python3 platform/macos/yu-shell-macos/run-resource-soak.py --documents 2 --seconds 30 --idle-seconds 70 artifacts/group4-next/soak-short-light
```

`/usr/bin/python3` 是本机已验证的解释器路径，其他测试机必须先确认自己的 Pillow 环境。完整多文档观测契约见 [资源压力入口](mac-group4-resource-soak.md)，重试预算的原始缺陷与八项回归见 [重试预算](mac-group4-retry-budget.md)。
