# `DocumentEditorSession` headless vertical slice benchmark（2026-08-14）

## 目的

在不启动 GUI 的情况下，沿 macOS 宿主实际使用的统一 Rust session 测量三条完整路径：

```text
UTF-8 文件 → DocumentEditorSession::open
           → source selection + InsertText command（local / random / fence-propagation）
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

输出现在拆成三个 workload：

- `local`：在普通段落内部连续插入中英文、emoji 和组合字符，不改变 block 边界。
- `random`：固定种子的随机范围替换，覆盖一般编辑和可能的 Markdown 结构变化。
- `fence-propagation`：在 1 MiB 未闭合 code fence 的开头删除 fence 标记，验证状态向 EOF
  传播的路径。

`--random-edits` 控制前两个 workload 的编辑次数；`fence-propagation` 固定为一次结构性编辑。

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

这组数字保留作 key-index 修复前后的历史对照；它把不同 Markdown 编辑类型混在一个平均值中，
不是当前 benchmark 的推荐读法。以上数字是本机诊断基线，不是最终产品性能承诺。

### 场景拆分与惰性 viewport（当前）

在修复 `ViewportLayout::sync` 的 key 索引后，又发现一个独立问题：即使从未请求过可视区域，
`map_through` 也会因为仅保存了 revision 而在第二次编辑时物化全文 block entries。现在 revision
本身不再被视为已 materialize 的 viewport state，首次 `visible_blocks` 查询才建立条目。

1 MiB fixture、4 次编辑、4 次 open 的诊断结果如下：

```text
workload            open median   edit total   edit mean   command median
local                 60.776 ms      18.933 ms    4.733 ms       4.726 ms
random                62.639 ms      22.005 ms    5.501 ms       5.220 ms
fence-propagation     40.863 ms      25.636 ms   25.636 ms      25.630 ms
```

`local`/`random` 从约 55 ms/次降到约 5 ms/次；`fence-propagation` 仍保留约 25 ms 的向 EOF
状态传播成本。save/reload 仍分别约 18 ms/60 ms，属于当前 session 文件 I/O 与 clean reload
路径，不应与编辑 parser 成本混为一谈。以上数字是本机诊断基线，不是最终产品性能承诺。

## 正确性不变量

- 打开后的 snapshot 必须等于 fixture。
- 每个选择和插入都必须通过 `DocumentEditorSession` 的 Revision-bound API。
- 所有随机编辑完成后，session source 必须等于独立字符串模型。
- 编辑后 session 必须 dirty，save 后必须 clean。
- reload 后 source 必须等于已保存 source。
- 未发生 viewport 查询时，编辑不得物化全文 block entries；首次可视查询后才建立完整索引。
