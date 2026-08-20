# ADR-0087：macOS 文件通知与关闭前状态机保持窄边界

## 状态

已接受（2026-08-13）

## 背景

`yu-storage::DocumentSession` 已经能通过磁盘指纹判断文件是否改变，但产品壳仍需要处理两类
生命周期事件：macOS 文件通知通常是成组、重复或由原子 rename 产生的中间事件；用户关闭 dirty
文档时又必须在保存、丢弃和取消之间保持确定状态。如果把 FSEvents、DispatchSource、计时器或
AppKit alert 直接放进 session，就会让共享内核依赖 run loop，并产生多个 close/dirty 真源。

## 决策

- `yu-storage::FileWatchDebouncer` 只接收路径事件，按 quiet period 合并事件，并返回一次
  `FileWatchCheck`；它不创建线程、不读取文件，也不把通知当成“文件确实改变”的证明。
- 新增 `platform/macos/yu-storage-macos`，把 FSEvents flags 和 DispatchSource vnode flags 映射为
  `FileWatchReason`，再调用共享 debouncer。FSEvents 丢失历史、目录扫描或 mount/unmount 等不确定
  情况映射为 `Unknown`，要求 session 重新做指纹检查。
- 原生 shell 自己拥有 FSEvents/DispatchSource、Dispatch queue/timer 和关闭窗口；Rust 适配层只
  暴露无状态的 flag 转换加可测试 debouncer，不让可变 `DocumentSession` 跨后台线程。
- `CloseStateMachine` 只表示 `Open → Prompting → Closed` 生命周期。clean session 立即关闭；dirty
  session 请求 `SaveChanges`；dirty 且外部文件改变/消失时请求 `ExternalChange`。save/reload/discard
  的实际 I/O 仍由 shell 调用 `DocumentSession` 后显式报告成功、取消或冲突。
- `DocumentSession::close_request` 统一计算 dirty 与 external conflict，平台层不得自行复制
  revision、磁盘指纹或 Markdown source。

## 结果

文件通知、指纹判断、关闭提示和实际保存各自只有一个职责，macOS 之外可以复用 close 状态机与
debouncer。原子保存触发的 rename/modified 事件会被合并，随后仍由 session 判断保存结果，不会因
watcher 回调顺序而静默覆盖文件。所有状态转换都有 headless 测试，macOS 适配只需验证 native
flags 和真实 callback 生命周期。

## 限制

当前适配没有实现真正的 FSEventStream 或 DispatchSource 生命周期，也没有 AppKit 文档窗口、alert
或文件监听线程；这些属于下一项产品壳工作。事件 flag 常量只覆盖 Yu 需要的分类，路径过滤仍由
每个 session adapter 负责。
