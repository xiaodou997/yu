# 0001. 主题系统住在 yu-workspace

- 状态：接受
- 日期：2026-09-13
- 相关不变量：I5、E1

## 背景

macOS 视觉重构（M1）之前，产品颜色散在 `yu-workspace` 的一排自由函数与
`Appearance` 方法里：`viewport_block_background`、
`Appearance::{background, text, editor_decorations}`、`viewport_table_style`、
`viewport_code_role_color`（加 light/dark 两张私有表）、分隔线/引用竖条/破图框
三个私有函数、任务框与图片 fallback 的裸 `Rgba8::new`。每加一个颜色多一个函数，
「这一套颜色配不配」只能人肉对照；M3/M4 要动视觉（字号、行高、间距、圆角、
内容列宽）时，没有一处能一眼看完「这一外观长什么样」。

「产品选色住在 `yu-workspace`、平台只跨 ABI 传深/浅这一个事实」这条规矩当时
已经写在 `Appearance` 的文档与不变量 I5 的条文里，但**住下来的是一堆函数，
不是一张表**——规矩守住了形状，没守住住址。

## 决策

在 `yu-workspace` 建 `pub struct Theme`：

- **全部产品颜色收进一张表**，按外观构造两张：`Theme::light()` / `Theme::dark()`。
  代码高亮那 8 个角色收进 `CodeRolePalette`（同样两张），表格四个值收进
  `TableSceneStyle`（`Theme` 的一个字段）。数值是现状的原文搬迁，M1 不改任何
  一个像素；`theme_tables_pin_every_product_color` 用例把两种外观的每个值
  逐个点名，后续改动的 diff 就是设计稿。
- **唯一入口是 `Appearance::theme(self) -> Theme`**。「一个事实进来、一整套
  颜色出去」的门只开在这里；新增颜色先进 `Theme::light`/`dark`，再经这道门
  选表。绕开 `Theme` 另挑颜色等于开第二份实现，两端一定漂开。
- **旧名字全部保留、内部改为委托**：`viewport_block_background`、
  `viewport_code_role_color`、`viewport_table_style` 签名不动
  （`yu-markdown` 的文档注释指着 `viewport_block_background`），
  `Appearance::{background, text, editor_decorations}` 也不动
  （`yu-storage-ffi` 在用）。
- **排版 token 占位不同期接线**。`body_font_size`（17）、`line_height_ratio`、
  `heading_scale`、`block_spacing`（0 = 现状不加）、`corner_radius`（0 = 现状
  直角）、`content_column_limit`（860）作为预留字段进 `Theme`，取值一律是
  「不改变现状」的中性值，且**当前没有任何调用点**。依赖方向卡死了消费者：
  `yu-editor` 不依赖 `yu-workspace`（是反过来），这些 token 只可能由
  `yu-workspace` 自己读；M3/M4 要把它们送进排版链路，得走 `yu-editor` 已有
  的 config 门，而不是指望 `yu-editor` 反向引用 `Theme`。

## 后果

### 正面

- 一种外观 = 一个构造函数，两套配色并排可读；改视觉只动
  `Theme::light`/`dark`，diff 即设计稿。
- 「产品选色唯一住所」从口头规矩变成编译期可导航的事实：顺着
  `Appearance::theme` 一把就到表。
- 跨 crate 消费者零改动：`yu-storage-ffi`、`yu-markdown` 引用的名字与签名
  原样保留。

### 负面

- `Theme` 目前只有按外观选表一个真实调用路径，排版 token 是先占座不营业的
  字段——读代码的人会问「为什么无调用点」，答案写在了 `Theme` 的结构体
  文档与本文档里，但确实多了一层需要读的东西。

### 引入的新问题与对策

- **两张表会漂开的风险从「函数之间」挪进了「构造函数之间」**（浅色块间距
  改了、深色没改）。对策：排版 token 两种外观共用同一份中性值，由
  `theme_tables_pin_every_product_color` 用例对两种外观同时点名；M3/M4 若要让
  两外观 token 分叉，必须先改那个用例，diff 里看得见。
- **预留 token 被遗忘接线的风险**。对策：字段文档各自写明现状由谁供给
  （行高由平台经 viewport config、标题倍率在 `yu-editor` 的 `BlockStyleTable`），
  M3/M4 动到那一项时知道去哪扇门接线。
