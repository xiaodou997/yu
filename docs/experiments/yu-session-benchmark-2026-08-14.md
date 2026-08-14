# `DocumentEditorSession` headless vertical slice benchmark（2026-08-14）

## 目的

在不启动 GUI 的情况下，沿 macOS 宿主实际使用的统一 Rust session 测量一条完整路径：

```text
UTF-8 文件 → DocumentEditorSession::open
           → source selection + InsertText command
           → incremental Markdown parse
           → atomic save
           → clean reload
```

benchmark 位于 `tools/yu-bench/src/bin/yu-session-bench.rs`，临时文件写入系统临时目录，进程
退出时自动删除。它只依赖 `DocumentEditorSession`，不会复制一份编辑器 source。

## 用法

快速 smoke：

```bash
cargo run -p yu-bench --bin yu-session-bench -- \
  --size-mib 1 --iterations 1 --random-edits 1
```

压力 workload：

```bash
cargo run -p yu-bench --bin yu-session-bench -- \
  --size-mib 10 --iterations 8 --random-edits 64
```

随机编辑可能改变 Markdown fence 或 block 状态，因此会故意测到向后传播的最坏路径；默认只运行
4 次随机编辑，避免普通开发验证意外变成长任务。

## 当前基线

在本机对 1 MiB fixture（1,048,590 bytes）运行 2 次 open、4 次随机编辑得到：

```text
open median:       79.148 ms
edit total:        54.269 s
edit mean:         13.567 s
edit command median: 26.939 s
save:              19.306 ms
reload:            62.115 ms
```

单次随机编辑的 1 MiB smoke（`--iterations 1 --random-edits 1`）约 5 ms；多次随机编辑进入
长时间传播，说明后续性能阶段必须把“普通局部编辑”和“fence/state 向 EOF 传播”拆开测量，不能
只看一个平均值。这是当前实现的诊断基线，不是最终产品性能承诺。

## 正确性不变量

- 打开后的 snapshot 必须等于 fixture。
- 每个选择和插入都必须通过 `DocumentEditorSession` 的 Revision-bound API。
- 所有随机编辑完成后，session source 必须等于独立字符串模型。
- 编辑后 session 必须 dirty，save 后必须 clean。
- reload 后 source 必须等于已保存 source。

