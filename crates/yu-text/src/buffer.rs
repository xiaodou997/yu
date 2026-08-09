use std::sync::Arc;

use yu_core::{ByteOffset, Revision};

use crate::{AppliedTransaction, EditError, Transaction};

/// An immutable, cheaply cloneable view of one document revision.
#[derive(Clone, Debug)]
pub struct TextSnapshot {
    revision: Revision,
    text: Arc<str>,
}

impl TextSnapshot {
    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn len_bytes(&self) -> ByteOffset {
        ByteOffset::try_from(self.text.len()).unwrap_or(ByteOffset::new(u64::MAX))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// A mutable text facade whose backend is deliberately replaceable.
#[derive(Debug)]
pub struct TextBuffer {
    revision: Revision,
    text: Arc<str>,
}

impl TextBuffer {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            revision: Revision::INITIAL,
            text: Arc::from(text.into()),
        }
    }

    #[must_use]
    pub fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn snapshot(&self) -> TextSnapshot {
        TextSnapshot {
            revision: self.revision,
            text: Arc::clone(&self.text),
        }
    }

    pub fn apply(&mut self, transaction: &Transaction) -> Result<AppliedTransaction, EditError> {
        let applied = transaction.apply_to(self.revision, &self.text)?;
        self.revision = applied.change_set().after();
        self.text = Arc::clone(applied.result_text());
        Ok(applied)
    }
}

#[cfg(test)]
mod tests {
    use yu_core::{ByteOffset, TextRange};

    use super::*;
    use crate::{Edit, Transaction};

    #[test]
    fn snapshots_remain_stable_after_an_edit() {
        let mut buffer = TextBuffer::new("羽");
        let old = buffer.snapshot();
        let insert_at_end = TextRange::empty(ByteOffset::new(3));
        let transaction = Transaction::new(buffer.revision(), [Edit::new(insert_at_end, " Yu")]);

        buffer
            .apply(&transaction)
            .expect("valid transaction should apply");

        assert_eq!(old.as_str(), "羽");
        assert_eq!(old.revision(), Revision::INITIAL);
        assert_eq!(buffer.snapshot().as_str(), "羽 Yu");
        assert_eq!(buffer.revision(), Revision::new(1));
    }
}
