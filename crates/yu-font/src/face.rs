//! Face 身份表：把平台的原生 face 换成一个稳定的 [`FontFaceId`]。
//!
//! # 为什么它在这里而不在平台层
//!
//! `FontFaceId` 由 **shaper** 铸（shaping 时才知道 fallback 选中了哪个 face），
//! 由 **rasterizer** 消费（[`crate::GlyphRasterKey`] 拿它去查字形）。两者
//! **必须共用同一张表**——各铸各的，第二张表的 0 号 face 会被第一张表的
//! 消费者解释成它自己的 0 号 face，**表现是屏幕上画出来的字全是别的字，
//! 不 panic、不报错**。
//!
//! 在 S7 第七刀之前，这条约定只存在于 macOS 侧的一个方法上
//! （`CoreTextShaper::rasterizer()` 返回一个共享同一个 `Arc<Mutex<..>>` 的
//! 对象），**任何 trait 上都没有表达**。第二端照着 trait 各写一个就会踩中。
//!
//! 现在它是一个类型：[`SharedFaceTable`] 是拿到一张表的**唯一**方式，
//! 后端的 rasterizer 只能由它构造。这不能阻止谁去新建第二张表，但它把
//! 「共用」变成了默认路径，把「不共用」变成了一句要显式写出来的
//! `SharedFaceTable::new()`。
//!
//! # 平台自己的那一半留在平台
//!
//! 表里存什么由平台定（类型参数 `T`）。macOS 存的是 PostScript 名**加上
//! 触发它的样本字符**——因为私有 UI 字体（`.SFNS-Regular`）的名字无法反过来
//! 创建字体，栅格化必须重放同一次 fallback。那是 CoreText 独有的补丁，
//! 不该进共享 seam；DirectWrite 可以直接存 `IDWriteFontFace`。

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex};

use yu_core::FontFaceId;

/// 表上出的错。两种都不该在正常路径上发生，但都必须报出来而不是猜一个 face。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceTableError {
    /// 一个进程铸了超过 `u32::MAX` 个 face。
    IdOverflow,
    /// 持锁的线程 panic 了。
    Poisoned,
}

impl fmt::Display for FaceTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdOverflow => formatter.write_str("font face id overflowed"),
            Self::Poisoned => formatter.write_str("face table is poisoned"),
        }
    }
}

impl Error for FaceTableError {}

/// 「平台 face 的键 → 稳定 id」的唯一实现。
///
/// 键是平台自己选的字符串（macOS 用 PostScript 名）。同一个键第二次问回同一个
/// id——**id 的稳定性是缓存的前提**：`GlyphRasterKey` 与 atlas 都按它建键，
/// 同一个 face 换一个 id 就是整张 atlas 白建一遍。
#[derive(Debug)]
pub struct FaceTable<T> {
    next: u32,
    ids: BTreeMap<String, FontFaceId>,
    entries: Vec<T>,
}

impl<T> Default for FaceTable<T> {
    fn default() -> Self {
        Self {
            next: 0,
            ids: BTreeMap::new(),
            entries: Vec::new(),
        }
    }
}

impl<T> FaceTable<T> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 这个键的 id。第一次见到时才调 `describe` 建条目。
    ///
    /// `describe` 是惰性的：重建一份平台描述（macOS 那边要记样本字符与字重）
    /// 在命中已有 face 时是白做的，而 shaping 的每一个 run 都会问一次。
    pub fn id_for(
        &mut self,
        key: &str,
        describe: impl FnOnce() -> T,
    ) -> Result<FontFaceId, FaceTableError> {
        if let Some(face) = self.ids.get(key) {
            return Ok(*face);
        }
        let face = FontFaceId::from_raw(self.next);
        self.next = self.next.checked_add(1).ok_or(FaceTableError::IdOverflow)?;
        self.entries.push(describe());
        self.ids.insert(key.to_owned(), face);
        Ok(face)
    }

    #[must_use]
    pub fn entry(&self, face: FontFaceId) -> Option<&T> {
        self.entries.get(usize::try_from(face.get()).ok()?)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 一张被 shaper 与 rasterizer 共用的 [`FaceTable`]。
///
/// **后端的 rasterizer 只应由它构造**，见模块文档。克隆的是句柄，不是表。
#[derive(Debug)]
pub struct SharedFaceTable<T>(Arc<Mutex<FaceTable<T>>>);

impl<T> Default for SharedFaceTable<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for SharedFaceTable<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> SharedFaceTable<T> {
    /// 新建一张**独立**的表。
    ///
    /// 一个后端通常只该调用它一次（macOS 侧是一个 `OnceLock` 的进程单例）。
    /// 每多调用一次就多一张彼此不认识的 id 空间。
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(FaceTable::new())))
    }

    /// 见 [`FaceTable::id_for`]。
    pub fn id_for(
        &self,
        key: &str,
        describe: impl FnOnce() -> T,
    ) -> Result<FontFaceId, FaceTableError> {
        self.0
            .lock()
            .map_err(|_| FaceTableError::Poisoned)?
            .id_for(key, describe)
    }

    /// 读一个条目。用闭包而不是返回引用，是为了不让锁守卫逃出去。
    pub fn with_entry<R>(
        &self,
        face: FontFaceId,
        read: impl FnOnce(&T) -> R,
    ) -> Result<Option<R>, FaceTableError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| FaceTableError::Poisoned)?
            .entry(face)
            .map(read))
    }

    /// 两个句柄指的是不是同一张表。
    #[must_use]
    pub fn is_same_table(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_key_keeps_the_same_id() {
        let mut table = FaceTable::new();
        let first = table.id_for("A", || 1_u32).expect("id");
        let second = table.id_for("B", || 2_u32).expect("id");
        assert_ne!(first, second);
        assert_eq!(table.id_for("A", || 99_u32).expect("id"), first);
        assert_eq!(table.entry(first), Some(&1));
        assert_eq!(table.len(), 2, "命中已有 face 不该再建条目");
    }

    #[test]
    fn describe_is_not_called_on_a_hit() {
        let mut table = FaceTable::new();
        table.id_for("A", || 1_u32).expect("id");
        table
            .id_for("A", || panic!("命中已有 face 时不该重建描述"))
            .expect("id");
    }

    #[test]
    fn an_unknown_face_is_none_not_a_neighbour() {
        let mut table = FaceTable::new();
        table.id_for("A", || 1_u32).expect("id");
        assert_eq!(table.entry(FontFaceId::from_raw(7)), None);
    }

    /// 这条是模块文档那段话的可执行形态：**两张表的 0 号 face 不是同一个
    /// face，而它们的 id 一模一样**。共用一个句柄才对。
    #[test]
    fn two_tables_mint_colliding_ids_for_different_faces() {
        let left = SharedFaceTable::new();
        let right = SharedFaceTable::new();
        let a = left.id_for("Latin", || "Latin").expect("id");
        let b = right.id_for("Emoji", || "Emoji").expect("id");
        assert_eq!(a, b, "各铸各的必然撞号");
        assert_eq!(
            left.with_entry(b, |entry| *entry).expect("读"),
            Some("Latin"),
            "拿别的表的 id 来查，查到的是自己表里同号的那个 face"
        );
        assert!(!left.is_same_table(&right));

        let shared = left.clone();
        assert!(shared.is_same_table(&left));
        assert_eq!(
            shared.with_entry(a, |entry| *entry).expect("读"),
            Some("Latin")
        );
    }
}
