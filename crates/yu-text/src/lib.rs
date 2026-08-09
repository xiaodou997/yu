#![forbid(unsafe_code)]

//! Immutable text snapshots and transactional editing contracts.
//!
//! The current flat UTF-8 storage is a Phase 1 reference backend. Public APIs
//! intentionally avoid exposing it so a persistent tree can replace it.

mod buffer;
mod transaction;

pub use buffer::{TextBuffer, TextSnapshot};
pub use transaction::{
    AnchorMapError, AppliedTransaction, ChangeSet, Edit, EditError, TextChange, Transaction,
};
