# v1 归档

本目录保存 Yu v1 架构的全部设计文档，**仅作历史参考，不再维护，不再作为实现依据**。

| 项 | 值 |
| --- | --- |
| v1 终点 commit | `e8140be` |
| v1 终点 tag | `v1-final` |
| v1 完整代码状态 | 分支 `archive/v1-source-projection` |

当前有效的架构依据是：

- [`docs/architecture/overview-v2.md`](../architecture/overview-v2.md)
- [`docs/specs/invariants.md`](../specs/invariants.md)

## 内容

| 路径 | 说明 |
| --- | --- |
| `adr/` | 183 篇 v1 ADR，**全部标记为 superseded** |
| `architecture/overview.md` | v1 架构总览 |
| `architecture/markdown-parser.md` | v1 自研 Markdown parser 设计 |
| `architecture/text-buffer.md` | v1 Piece Tree / 文本存储设计 |
| `specs/invariants.md` | v1 核心不变量（659 行） |
| `roadmap/phase-1..3.md` | v1 阶段路线 |
| `experiments/` | v1 风险实验记录；其中的命令路径反映当时的目录结构 |

## 为什么整体归档

这批文档中相当大的比例在描述 v2 已删除的机制：TextKit fail-closed fallback、
capability mask / block kind mask、coverage 查询、visual render state machine、
count/fill C ABI。留在活跃目录会持续误导人和 AI 协作者。

历史价值由本目录与 git 历史承载即可。v2 的 ADR 从 `docs/adr/0001` 重新开始编号。

归档动因的完整分析见 [`overview-v2.md` 第 2 节](../architecture/overview-v2.md)。
