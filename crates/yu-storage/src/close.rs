use super::ExternalFileState;

/// Prompt required before closing a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClosePrompt {
    SaveChanges,
    ExternalChange { state: ExternalFileState },
}

/// Observable state of the close lifecycle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CloseState {
    #[default]
    Open,
    Prompting(ClosePrompt),
    Closed,
}

/// Result of asking the state machine to close.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseRequest {
    CloseNow,
    Prompt(ClosePrompt),
    AlreadyClosed,
}

/// State transition after the product shell handles a close prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseTransition {
    Cancelled,
    Closed,
    Prompt(ClosePrompt),
}

/// Invalid action for the current close lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseStateError {
    AlreadyClosed,
    NotPrompting,
}

/// Platform-neutral close-before-discard state machine.
///
/// It does not save or reload by itself. The product shell calls
/// `DocumentSession::save`, `reload`, or an explicit discard policy and then
/// reports the result here. This keeps file I/O and UI prompts separate while
/// making close behavior deterministic in headless tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CloseStateMachine {
    state: CloseState,
}

impl CloseStateMachine {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: CloseState::Open,
        }
    }

    #[must_use]
    pub const fn state(self) -> CloseState {
        self.state
    }

    /// Requests close using the session's current dirty and external-change
    /// facts. A clean session closes immediately, including when only the disk
    /// copy changed: there is no local edit that would be lost.
    pub fn request_close(
        &mut self,
        dirty: bool,
        external_change: Option<ExternalFileState>,
    ) -> CloseRequest {
        match self.state {
            CloseState::Closed => CloseRequest::AlreadyClosed,
            CloseState::Prompting(prompt) => CloseRequest::Prompt(prompt),
            CloseState::Open => {
                if !dirty {
                    self.state = CloseState::Closed;
                    CloseRequest::CloseNow
                } else {
                    let prompt = match external_change {
                        Some(state) => ClosePrompt::ExternalChange { state },
                        None => ClosePrompt::SaveChanges,
                    };
                    self.state = CloseState::Prompting(prompt);
                    CloseRequest::Prompt(prompt)
                }
            }
        }
    }

    pub fn cancel(&mut self) -> Result<CloseTransition, CloseStateError> {
        match self.state {
            CloseState::Prompting(_) => {
                self.state = CloseState::Open;
                Ok(CloseTransition::Cancelled)
            }
            CloseState::Open => Err(CloseStateError::NotPrompting),
            CloseState::Closed => Err(CloseStateError::AlreadyClosed),
        }
    }

    /// Marks the session closed after the caller successfully saved (or
    /// otherwise resolved the conflict and persisted the source).
    pub fn save_succeeded(&mut self) -> Result<CloseTransition, CloseStateError> {
        match self.state {
            CloseState::Prompting(_) => {
                self.state = CloseState::Closed;
                Ok(CloseTransition::Closed)
            }
            CloseState::Open => Err(CloseStateError::NotPrompting),
            CloseState::Closed => Err(CloseStateError::AlreadyClosed),
        }
    }

    /// Discards local changes and marks the session closed. The caller must
    /// ensure that any external file remains untouched; `yu-storage` never
    /// overwrites it during this transition.
    pub fn discard(&mut self) -> Result<CloseTransition, CloseStateError> {
        match self.state {
            CloseState::Prompting(_) => {
                self.state = CloseState::Closed;
                Ok(CloseTransition::Closed)
            }
            CloseState::Open => Err(CloseStateError::NotPrompting),
            CloseState::Closed => Err(CloseStateError::AlreadyClosed),
        }
    }

    /// Converts a failed save caused by an external change into the stronger
    /// conflict prompt without closing the session.
    pub fn save_failed_external(
        &mut self,
        state: ExternalFileState,
    ) -> Result<CloseTransition, CloseStateError> {
        match self.state {
            CloseState::Prompting(_) => {
                let prompt = ClosePrompt::ExternalChange { state };
                self.state = CloseState::Prompting(prompt);
                Ok(CloseTransition::Prompt(prompt))
            }
            CloseState::Open => Err(CloseStateError::NotPrompting),
            CloseState::Closed => Err(CloseStateError::AlreadyClosed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_close_is_immediate_and_idempotent() {
        let mut machine = CloseStateMachine::new();
        assert_eq!(machine.request_close(false, None), CloseRequest::CloseNow);
        assert_eq!(machine.state(), CloseState::Closed);
        assert_eq!(
            machine.request_close(false, None),
            CloseRequest::AlreadyClosed
        );
    }

    #[test]
    fn dirty_close_can_cancel_then_save_or_discard() {
        let mut machine = CloseStateMachine::new();
        assert_eq!(
            machine.request_close(true, None),
            CloseRequest::Prompt(ClosePrompt::SaveChanges)
        );
        assert_eq!(machine.cancel(), Ok(CloseTransition::Cancelled));
        assert_eq!(
            machine.request_close(true, None),
            CloseRequest::Prompt(ClosePrompt::SaveChanges)
        );
        assert_eq!(machine.save_succeeded(), Ok(CloseTransition::Closed));

        let mut discard = CloseStateMachine::new();
        discard.request_close(true, None);
        assert_eq!(discard.discard(), Ok(CloseTransition::Closed));
    }

    #[test]
    fn external_change_prompts_conflict_and_does_not_auto_close() {
        let mut machine = CloseStateMachine::new();
        let prompt = ClosePrompt::ExternalChange {
            state: ExternalFileState::Missing,
        };
        assert_eq!(
            machine.request_close(true, Some(ExternalFileState::Missing)),
            CloseRequest::Prompt(prompt)
        );
        assert_eq!(
            machine.save_failed_external(ExternalFileState::Changed),
            Ok(CloseTransition::Prompt(ClosePrompt::ExternalChange {
                state: ExternalFileState::Changed,
            }))
        );
        assert_eq!(
            machine.state(),
            CloseState::Prompting(ClosePrompt::ExternalChange {
                state: ExternalFileState::Changed,
            })
        );
    }

    #[test]
    fn actions_are_rejected_outside_prompt() {
        let mut machine = CloseStateMachine::new();
        assert_eq!(machine.cancel(), Err(CloseStateError::NotPrompting));
        assert_eq!(
            machine.save_failed_external(ExternalFileState::Changed),
            Err(CloseStateError::NotPrompting)
        );
    }
}
