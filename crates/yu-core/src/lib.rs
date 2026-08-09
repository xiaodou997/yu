#![forbid(unsafe_code)]

//! Stable foundational types shared by Yu Editor's core crates.

mod position;

pub use position::{Affinity, ByteOffset, Revision, TextAnchor, TextRange};
