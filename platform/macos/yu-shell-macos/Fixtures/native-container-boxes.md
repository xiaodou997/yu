# 容器内的内容边界

> 引用正文与下面的表格、代码应共用左边界。
>
> | 项目 | 描述 |
> | --- | --- |
> | 排版 | 表格限制在引用容器的可用宽度中，长文字在单元格内换行。 |
>
> ```rust
> let width = container.available_width();
> println!("原生排版");
> ```

- 列表正文

  | 项目 | 描述 |
  | --- | --- |
  | 几何 | 表格跟随列表缩进 |

  ```swift
  let text = "Swift + CoreText"
  ```

> ![本地图片](assets/yu-mark.png)

正文结束。
