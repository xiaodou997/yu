# Yu 验收脚本

统一入口：

```sh
python3 tools/run_acceptance.py group4-fixed artifacts/新目录
```

`run_acceptance.py` 是所有功能共用的执行器。它创建独立输出目录、串行执行命令、保存原始日志和退出码，并根据必需证据层计算每个用例的状态。第四组的固定输入、预期和日志判定只在 `group4_fixed.py` 中。旧命令 `run_group4_fixed_acceptance.py` 只是兼容入口。

## 新功能接入

在本目录新增 `feature_name.py`，实现 `run_suite(ctx) -> int`。命令行名称用连字符，例如 `feature-name` 对应 `feature_name.py`。一个最小套件：

```python
from acceptance_runner import write_json


def run_suite(ctx):
    code = ctx.run("core", ["cargo", "test", "--locked", "-p", "yu-editor", "new_feature"])
    ctx.ledger.add("new-feature/example", ["core", "real_window"])
    # 先核对测试输出中属于该用例的明确成功证据，再记录本层结果。
    if code == 0 and "new-feature/example PASS" in ctx.log("core").read_text():
        ctx.ledger.record("new-feature/example", "core", "passed", "core")
    rows = ctx.ledger.finish()
    write_json(ctx.output / "reports" / "cases.json", {"cases": rows})
    return 0 if code == 0 and rows[0]["status"] == "partial" else 1
```

套件自己负责生成输入、核对文件哈希、确认每个 ID 实际完成了对应断言，并定义必需证据层。通用执行器拒绝重复用例、重复步骤、重复证据层以及用失败命令记通过；只有全部必需层通过才产生 `passed`。未做真实窗口检查时应保留 `partial`，不要把核心测试的通过标记同时用于窗口层。

真实窗口也用同一个 `ctx.run(...)` 串行启动现有桌面驱动。套件需核对驱动的 `results.json`、用例 ID、截图与保存结果，再用 `ctx.artifact(path)` 保存证据文件的路径和 SHA256，最后记录 `real_window` 或 `cold_reopen` 层。`ctx.run` 不会因为命令返回 0 就自动给每个用例通过。

已有证据层：`core`、`native`、`real_window`、`cold_reopen`、`system_event`、`visual`。套件可针对功能选择需要的层，不必强制所有功能跑同一套组合。输出目录必须是全新目录；已有结果不会被覆盖。
