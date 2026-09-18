# Native paragraph acceptance 原生段落验收

正文 English 中文，标点不能出现在错误的位置。**粗体**、*斜体*、`inline code` 和 [链接](https://example.com) 应共享正确的基线。

## 立即开始

如果你第一次接触 `md2wechat`，先按这个顺序走：

- 如果你是 mac 用户，优先用 Homebrew 安装 CLI：`brew install geekjourneyx/tap/md2wechat`
- 如果你已经有稳定可用的 Go 环境，也可以选 `go install github.com/geekjourneyx/md2wechat-skill/cmd/md2wechat@v2.0.5`
- 想直接安装 CLI：看 [安装指南](https://example.com/install)

### Nested containers

1. 第一项
   - 内层列表
   - another child
2. 第二项

> 引用中的第一段。
>
> - 引用列表
> - 第二项

- 第一段列表项。

  同一个列表项里的第二段。

- 第二项。

```swift
// 中文注释
let greeting = "Hello, 世界 👨‍👩‍👧‍👦"
print(greeting)
```

| Name | Value |
| :--- | ---: |
| 羽 | 123 |
| English | **bold** |

- [ ] 未完成
- [x] 已完成

Arabic العربية, Hebrew עברית, English. Ligatures office affinity; combining é; emoji 👨‍👩‍👧‍👦 🇦🇺.



连续空行后的段落。源码空白必须保留，阅读布局不得机械放大。

YU_END_OF_DOCUMENT
