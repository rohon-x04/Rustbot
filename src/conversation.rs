//! Sequential conversations: ask a question, wait for the user's next message.

use std::time::Duration;

use crate::client::Client;
use crate::error::{Error, Result};
use crate::filters::message::MessageFilter;
use crate::types::{Message, Update};

/// Options for [`Client::ask`] / [`Client::ask_from`].
#[derive(Clone)]
pub struct WaitMessageOpts {
    /// Only messages matching this filter are accepted.
    pub filter: Option<MessageFilter>,
    /// A message matching this filter ends the wait with
    /// `Error::ConversationCancelled` (e.g. the user sends `/cancel`).
    pub cancellation_filter: Option<MessageFilter>,
    pub timeout: Duration,
}

impl Default for WaitMessageOpts {
    fn default() -> Self {
        WaitMessageOpts { filter: None, cancellation_filter: None, timeout: Duration::from_secs(60) }
    }
}

impl WaitMessageOpts {
    pub fn timeout(secs: u64) -> WaitMessageOpts {
        WaitMessageOpts { timeout: Duration::from_secs(secs), ..Default::default() }
    }
}

impl Client {
    /// Wait for the next message in `chat_id` (from anyone).
    pub fn ask(&self, chat_id: i64, opts: Option<WaitMessageOpts>) -> Result<Message> {
        self.ask_inner(chat_id, 0, opts)
    }

    /// Wait for the next message in `chat_id` from `user_id` only.
    pub fn ask_from(&self, chat_id: i64, user_id: i64, opts: Option<WaitMessageOpts>) -> Result<Message> {
        if user_id == 0 {
            return Err(Error::InvalidUserId);
        }
        self.ask_inner(chat_id, user_id, opts)
    }

    fn ask_inner(&self, chat_id: i64, user_id: i64, opts: Option<WaitMessageOpts>) -> Result<Message> {
        let opts = opts.unwrap_or_default();
        let timeout = if opts.timeout.is_zero() { Duration::from_secs(60) } else { opts.timeout };
        let (filter, cancel) = (opts.filter.clone(), opts.cancellation_filter.clone());
        let update = self.wait_for_chat(
            chat_id,
            move |_c: &Client, u: &Update| {
                if !u.is("updateNewMessage") {
                    return false;
                }
                let m = match u.message() {
                    Some(m) => m,
                    None => return false,
                };
                if m.chat_id() != chat_id {
                    return false;
                }
                if user_id != 0 && m.sender_id() != user_id {
                    return false;
                }
                if let Some(c) = &cancel {
                    if c.test(&m) {
                        return true;
                    }
                }
                filter.as_ref().map_or(true, |f| f.test(&m))
            },
            timeout,
        )?;
        let msg = update.message().ok_or(Error::NoFormattedText)?;
        if let Some(c) = &opts.cancellation_filter {
            if c.test(&msg) {
                return Err(Error::ConversationCancelled);
            }
        }
        Ok(msg)
    }
}
