//! A deployment owner, shared by ingress, durable operations and background delivery.

/// A positive Telegram user ID representable as a private-chat ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Owner(i64);

impl Owner {
    /// Reject invalid IDs before an integration can start.
    pub fn new(user: u64) -> Option<Self> {
        i64::try_from(user).ok().filter(|id| *id > 0).map(Self)
    }

    /// Accept only the owner acting in their own private chat.
    pub fn authorizes(self, user: u64, chat: i64, private: bool) -> bool {
        private && i64::try_from(user).ok() == Some(self.0) && self.chat(chat)
    }

    pub(crate) fn chat(self, chat: i64) -> bool {
        chat == self.0
    }
}

#[cfg(test)]
#[path = "authorization_tests.rs"]
mod tests;
