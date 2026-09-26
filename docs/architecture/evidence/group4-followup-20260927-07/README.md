# 第七轮验证证据

对应报告：[撤销入口与资源初始化所有权](../../mac-group4-followup-07.md)。本目录只保留本轮已经完成的记录；第四组不结项。

- `history-routing-packaged-before.log`：旧产品逻辑加最初 8 个原生路由场景，4 个失败。`history-routing-after.log`：修正后的 12 个场景通过；属于原生入口检查，不是外部事件投递复现。
- `bridge-resources-before.log`／`bridge-resources-after.log`：不完整隔离包的初始化失败测试，前者 1 项失败，后者 2 项通过。`initialization-crash-summary.json` 是本次早期独立产物崩溃的有限栈摘要，不包含整机诊断内容。
- `native-final.log`：当前正式构建 19 项原生自检（内部包含路由场景）及 2 项初始化失败子进程检查通过。`tool-tests-final.log`：73 项既有工具／原生进程测试通过；这些数字不重复累加。
- `baseline-dark-result.json`：旧构建有限场景，不计作修正后验收。`ime-dark-result.json`、`dark-soak-result.json`、`cold-light-result.json` 是同一新构建的三组实窗结果。
- 三张 PNG 已实际复核。`build-final-manifest.json` 和 `final-source-lock.json` 绑定构建与源码；测试隔离包重新签名后的二进制哈希与生产包不同，各结果均单独记录。

`summary.json` 记录文件哈希和归因边界。原始日志、实验应用和完整事件仍在本机 `artifacts/group4-local-20260927-07/`。原始偶发撤销失败仍未取得唯一因果链，定向入口修复和有限实窗成功不能抹去该限制。
