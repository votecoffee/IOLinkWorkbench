//! Normal GUI close waits for job completion without blocking the UI thread.
use super::App;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CloseAction {
    Idle,
    Wait,
    Cancel,
    Close,
}

impl App {
    pub(super) fn settle_close(&mut self, requested: bool, capture_failed: bool) -> CloseAction {
        if capture_failed && (self.closing || requested) {
            self.closing = false;
            self.stop.request();
            self.message = format!("Close canceled · {}", self.message);
            return CloseAction::Cancel;
        }
        self.advance_close(requested)
    }

    pub(super) fn pending_jobs(&self) -> usize {
        usize::from(self.worker.is_some())
            + usize::from(self.isdu_worker.is_some())
            + usize::from(self.setup.busy())
            + self.saved_capture.pending_jobs()
            + usize::from(self.saved_parameter.busy())
    }

    pub(super) fn advance_close(&mut self, requested: bool) -> CloseAction {
        if requested {
            self.closing = true;
            self.stop.request();
        }
        if !self.closing {
            return CloseAction::Idle;
        }
        let pending = self.pending_jobs();
        if pending == 0 {
            CloseAction::Close
        } else {
            self.message = format!("Closing · waiting for {pending} pending operations to finish…");
            CloseAction::Wait
        }
    }
}

#[cfg(test)]
mod tests;
