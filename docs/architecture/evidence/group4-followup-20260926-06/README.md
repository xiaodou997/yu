# 第六轮证据边界

报告入口：[mac-group4-followup-06.md](../../mac-group4-followup-06.md)。本目录保留已结束运行的原始结果副本及定向回归，不修改它们的 `passed` 字段来追溯更正；最终结论以报告中明确区分的维度为准。

- `control-600-light.json`：旧入口不插入 vmmap 的四文档600秒对照。源码／历史／公式像素和应用物理足迹证据保留；旧 helper 字符串匹配漏报，helper 数量、足迹和退出断言不能按其中 `passed=true` 签收。
- `process-identity-observation.json`：同一时刻，4个实际 helper 的启动命令与旧解析器的空列表对照，路径均属于隔离测试应用。
- `kernel-short-light.json`：修正内核可执行路径观察后的完整双文档短场景。
- `first-1800-failed.json`：第一次30分钟尝试的原始失败结果，实际只完成3轮；不算30分钟通过，也不因后续重跑而删除。
- `kernel-inventory-before.log`／`kernel-inventory-after.log`：实际进程回归修改前失败与修复后的工具测试。`postmortem-tests.log` 为之后增加失败快照顺序回归的结果。这些测试不能累加成产品实窗次数。

`kernel-1800-light-evidence.json` 是增强失败快照后的独立长场景：85轮、实际压力1805.826秒、整套1995.542秒并正常结束。`long-soak-helper-summary.json` 从该次 `events.jsonl` 汇总，原始事件SHA256为 `ffb9a4dba3557de60c6da5733b876b7df08ddfd05f60e43a67e86fc8d852297f`。`long-soak-cold-reopen.png` 是实际查看的终点截图。后续通过不抹掉 `first-1800-failed.json`，不代表先前撤销未生效的原因已经确定或修复。

每个原始 JSON 自带产品、脚本和原始日志哈希；完整事件、源码不匹配原文和本轮截图仍位于测试机 `artifacts/group4-local-20260926-06/`。当前产品二进制未因测试工具修改而重新打包。最终内存归因、真实日历事件和第四组完整结项仍需单独证据。
