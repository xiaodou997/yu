# 第五组预期警告语料

这些输入的预期是可见诊断、明确确认后 completed_with_warnings，而不是普通成功。

<script>alert('不得执行')</script>

<img src="missing.png" onerror="alert('不得执行')">

[危险链接](javascript:alert%281%29)

![远程图片](https://example.invalid/never-fetch.png)

![缺失图片](assets/group5-deliberately-missing.png)

$$
\unknownYuCommand{x}
$$

```mermaid
yuUnsupportedGraph
A -> B
```

<iframe src="https://example.invalid/never-load"></iframe>

GROUP5-WARNING-END-警告之后的正文不能丢失。
