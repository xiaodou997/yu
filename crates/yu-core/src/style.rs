//! 排版样式请求。
//!
//! 这个枚举此前叫 `VisualRunStyle`，定义在 `yu-projection` 里，于是 `yu-font`
//! 为了实现 `ClusterMetrics` 不得不反向依赖投影层。它描述的是「这段文本按什么
//! 字型排」，与 Markdown 无关：斜体、粗体、等宽是排版概念，任何来源的文本都
//! 可能带上它们。
//!
//! S4 引入 `yu-decoration` 之后它会并入 `StyleId`；在那之前它留在 `yu-core`，
//! 让 `yu-layout` 与 `yu-font` 都只依赖同一个下游 crate。

/// 一段文本的字型请求。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextStyle {
    #[default]
    Plain,
    Emphasis,
    Strong,
    Code,
}
