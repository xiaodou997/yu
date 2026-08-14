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

原始实现对 1 MiB fixture（1,048,590 bytes）运行 2 次 open、4 次随机编辑时得到：

```text
open median:       79.148 ms
edit total:        54.269 s
edit mean:         13.567 s
save:              19.306 ms
reload:            62.115 ms
```

瓶颈来自 `ViewportLayout::sync` 通过旧 entries 对每个新 block 做线性查找，多个编辑后变成
O(blocks²)。改为 `ViewportKey → entry` 哈希索引后，同一 workload 得到：

```text
open median:              88.469 ms
edit total:              222.654 ms
edit mean:                55.664 ms
selection median:        243.333 µs
command median:           98.683 ms
selection + command:      99.064 ms
save:                      26.380 ms
reload:                    60.595 ms
```

单次随机编辑的 1 MiB smoke（`--iterations 1 --random-edits 1`）仍约 5 ms。后续性能阶段仍需
把“普通局部编辑”和“fence/state 向 EOF 传播”拆开测量，不能只看一个平均值；以上数字是本机
诊断基线，不是最终产品性能承诺。

## 正确性不变量

- 打开后的 snapshot 必须等于 fixture。
- 每个选择和插入都必须通过 `DocumentEditorSession` 的 Revision-bound API。
- 所有随机编辑完成后，session source 必须等于独立字符串模型。
- 编辑后 session 必须 dirty，save 后必须 clean。
- reload 后 source 必须等于已保存 source。
