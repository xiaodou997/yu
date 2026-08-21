# CommonMark spec 用例

## 这是什么

`spec.json` 是 CommonMark 规范 0.31.2 的全部 652 条示例，每条含 `markdown`
输入、期望的 `html` 输出、所属章节与它在 `spec.txt` 里的行号。

不变量 C7 规定「CommonMark 语义以官方 spec 用例为准」。这份文件就是那个「为准」
的东西，`crates/yu-syntax/tests/commonmark_spec.rs` 逐条跑它。

## 出处与校验

| 项 | 值 |
| --- | --- |
| 上游 | <https://spec.commonmark.org/0.31.2/spec.json> |
| 版本 | 0.31.2 |
| 用例数 | 652 |
| SHA-256 | `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20` |
| 取得日期 | 2026-08-21 |

校验和由测试在每次运行时核对（见 `commonmark_spec.rs`）。**入库而不是在测试时
下载**：CI 跑在三个平台上，一次网络抖动就会让一条以退出码为准的门禁变成
掷骰子；而「规范用例悄悄换了一版」是比网络失败严重得多的事情，必须是一次
显式的、带 diff 的提交。

## 许可

规范文本与由它生成的用例是 **CC-BY-SA 4.0**，版权归 John MacFarlane，
全文见同目录的 `LICENSE`。仓库其余部分是 Apache-2.0；这份文件不适用那条许可，
也不参与产物构建——它只被 `yu-syntax` 的测试读取。

## 怎么升级

1. 下载新版 `spec.json`，替换本目录下的同名文件；
2. 更新上表的版本、用例数、SHA-256 与日期；
3. 更新 `commonmark_spec.rs` 里的 `EXPECTED_EXAMPLE_COUNT` 与 `SPEC_SHA256`；
4. 跑一遍测试，把新增或改变的偏差逐条登记进
   `docs/specs/invariants.md` 第 F 节。

第 4 步不能跳过。规范每一版都会调整边角语义，通过率的变化必须被逐条解释，
而不是把阈值往下调。
