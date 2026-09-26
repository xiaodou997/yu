# 第四组：嵌入资源重试预算不能在重新排队时归零

更新：2026-09-26。开发基线：`f3762f87f63dc08994a171f9821d8b36327dd550`。

## 当前顺序与范围

开始本批时，远端 main 已包含 `f3762f87` 的末帧丢弃恢复修复。该提交记录了浅色八次、深色八次冷重开像素／历史检查通过，以及其自身的 Rust、原生与工具检查。这些是已有提交的记录，不是本批重新执行的结果；原 600 秒压力和真实长时多文档验收仍须在对应构建上补齐。

本批继续资源稳定性工作，修复一个在代码检查中确认的重试预算缺陷。没有撤销或修改 `f3762f87`；没有修改 Metal、着色器、缓存容量、系统时间、CI、导出或历史冻结标签。第四组仍未结项。

## 缺陷与最小修复

`crates/yu-assets/src/embedded.rs` 的 `EmbeddedResourceCache::request` 在同一 revision 的可重试失败到期后，先删除 `failures` 记录，再把请求加入队列。随后 `complete -> record_failure` 从这张表读取前次尝试次数；记录已被删除，因此新的失败又被计为第 1 次。

例如默认策略是最多 3 次失败、初始退避 2 tick。按真实 `request -> pending -> complete` 顺序反复失败，旧实现会不断得到 `attempts = 1`、退避 2 tick，而不是递增到 2、3 次后停止。只要宿主继续推进重试时钟并请求资源，这条路径就能绕过预期的次数上限。静态调用链已核对到 `MacosEmbeddedResourceState::request_result`，不是一段未接入产品的辅助代码。

修复仅移除到期重试分支中的提前删除，保留同一 revision 的失败元数据：

- 排队和重复状态查询不消耗、也不重置预算；仍由现有 pending/in-flight 去重。
- 每次实际失败完成时，沿用 `record_failure` 递增次数并计算退避；耗尽后保持失败，不再排队。
- 成功仍由 `publish` 清除失败记录；revision 改变仍按既有清理规则取得新预算。
- 旧 revision 的成功／失败仍拒绝，不能删除新请求所有权或重置新版本的计数。
- 非法源码与不支持类型不自动重试；不同资源和不同文档缓存的预算独立。

这修复的是重试状态机，不是已经测得的进程内存占用结论。不能把早先 300 秒／600 秒压力中的内存增长全部归因于它，也不能因此宣布长期无泄漏。

## 新增回归

`crates/yu-assets/tests/embedded_retry_lifecycle.rs` 新增 8 个集成测试函数，全部走公开的请求、队列和完成接口，不直接反复调用 `record_failure` 冒充完整重试链路，也不依赖桌面、真实 helper 或墙上时钟：

| 测试 | 断言 |
| --- | --- |
| 默认重试链路 | Worker／Render 失败次数为 1、2、3，退避递增，耗尽后持续查询不再排队 |
| 退避上限 | 指数退避到顶后保持上限，但失败次数继续累计 |
| 重复查询 | 排队期间更新源码范围，执行期间不重复派发，不丢失前次失败记录 |
| 重试成功 | 清理失败记录，保留成功缓存，跨 revision 可重新绑定原结果 |
| 旧结果晚到 | 旧成功／失败不污染新 revision 的重试计数或 in-flight 所有权 |
| 不可重试错误 | InvalidSource／Unsupported 始终不自动排队 |
| 边界策略 | 零延迟和单次上限仍能停止，零次数参数沿用既有最小值规则 |
| 隔离 | 一个资源耗尽不消耗其他资源或其他文档缓存的预算 |

## 本批验证边界

已核对修改前文件的完整 Git blob SHA 为 `c51cce82919fc6e2599ba1b7d979ca97fe78a650`，并检查产品差异仅为删除一行提前清理、增加解释注释。新增测试逐项对照公开 API 和生产调用链检查。

**当前 Linux 编辑环境没有 cargo/rustc。本批未执行上述 8 项 Rust 测试、cargo fmt、Clippy、macOS 构建或实窗压力，不能写成“8 项通过”或“修复前红／修复后绿”。** 上一批 33 项工具测试及 `f3762f87` 的结果均不能计作本批测试。

测试机拉取当前 main 后，从仓库根目录执行并保留各命令退出码：

```sh
cargo test --locked -p yu-assets --test embedded_retry_lifecycle
cargo test --locked -p yu-assets -p yu-embedded-client -p yu-storage-ffi
cargo fmt --all --check
cargo clippy --locked -p yu-assets -p yu-embedded-client -p yu-storage-ffi --all-targets -- -D warnings
```

完成这些检查并重新构建、审计 Release 后，先复测已有冷重开入口，再运行多文档短场景。必须使用新输出目录，核对实际应用／helper 哈希；原有像素断言、失败证据和源码／历史检查均不得跳过：

```sh
python3 platform/macos/yu-shell-macos/run-embedded-checks.py \
  --cold-resource-reopen --reopen \
  artifacts/group4-retry-budget/cold-light
python3 platform/macos/yu-shell-macos/run-embedded-checks.py \
  --cold-resource-reopen --reopen --dark \
  artifacts/group4-retry-budget/cold-dark
python3 platform/macos/yu-shell-macos/run-resource-soak.py \
  --documents 2 --seconds 30 --idle-seconds 70 \
  artifacts/group4-retry-budget/short-light
```

以上均为待执行命令，不是已有本批结果。当前有效／无效／普通语料压力不会稳定制造 Worker／Render 瞬时失败，不能仅靠该压力场景证明重试预算正确；预算回归由上面的定向测试负责，实际进程生命周期与像素仍由真机补验。

长时多文档的分阶段足迹入口见 [多文档资源压力](mac-group4-resource-soak.md)。内存最终归因、原长压力复测、真实跨午夜／睡眠／时区事件，以及完整选区与长页组合仍开放。

## 后续本机验证（独立于上述初次提交）

2026-09-26 通过用户授权的本机连接完成八项定向回归，以及包含它们的136项相关Rust测试、Clippy和全仓格式检查。格式调整和外观测试修正已在 `932f0154` 提交；新Release完成浅深色冷重开及实际多文档压力。新增像素外观门禁后60项工具测试通过。详细受测身份、最初浅色误标、四文档600秒实际结果与仍存在的内存归因边界见 [收尾第五轮](mac-group4-followup-05.md)。这些结果不回写成初次Linux提交时已经执行，也不代表第四组结项。
