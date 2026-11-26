//! Update handlers.
//!
//! Handlers live in numbered *groups*. Groups run in ascending order and inside
//! a group handlers run in registration order. The first handler whose
//! `check_update` matches runs; afterwards dispatch **stops** unless the handler
//! returns a control error:
//!
//! * `Err(Error::ContinueHandlers)` – keep trying the next handler in this group
//! * `Err(Error::ContinueGroups)` – skip to the next group
//! * `Err(Error::StopPropagation)` – stop everything (same as returning `Ok(())`,
//!   but also skips the error handler)

use std::sync::Arc;

use crate::client::Client;
use crate::command::parse_command;
use crate::error::HandlerResult;
use crate::filters::callback::CallbackFilter;
use crate::filters::message::MessageFilter;
use crate::types::{CallbackQuery, Message, Update};

/// The interface of every update handler.
pub trait Handler: Send + Sync {
    /// Should this handler process `update`?
    fn check_update(&self, client: &Client, update: &Update) -> bool;
    /// Process `update`.
    fn handle_update(&self, client: &Client, update: &Update) -> HandlerResult;
}

/// What kinds of messages a handler accepts.
#[derive(Clone, Copy, Debug)]
pub struct MatchingOptions {
    pub allow_outgoing: bool,
    pub allow_incoming: bool,
    pub allow_edited: bool,
    pub allow_channel: bool,
    pub allow_group: bool,
    pub allow_private: bool,
}

impl Default for MatchingOptions {
    fn default() -> Self {
        MatchingOptions {
            allow_outgoing: false,
            allow_incoming: true,
            allow_edited: true,
            allow_channel: true,
            allow_group: true,
            allow_private: true,
        }
    }
}

impl MatchingOptions {
    pub fn check(&self, m: &Message) -> bool {
        if m.is_outgoing() && !self.allow_outgoing {
            return false;
        }
        if !m.is_outgoing() && !self.allow_incoming {
            return false;
        }
        if m.is_edited() && !self.allow_edited {
            return false;
        }
        if m.is_supergroup_or_channel() && !self.allow_channel {
            return false;
        }
        if m.is_group() && !self.allow_group {
            return false;
        }
        if m.is_private() && !self.allow_private {
            return false;
        }
        true
    }
}

type MessageFn = Box<dyn Fn(&Client, &Message) -> HandlerResult + Send + Sync>;
type CallbackFn = Box<dyn Fn(&Client, &CallbackQuery) -> HandlerResult + Send + Sync>;
type UpdateFn = Box<dyn Fn(&Client, &Update) -> HandlerResult + Send + Sync>;

/// Handles `/command` messages.
pub struct CommandHandler {
    pub options: MatchingOptions,
    /// Accepted prefixes (default `["/"]`).
    pub triggers: Vec<String>,
    /// Lower-cased command name without prefix.
    pub command: String,
    pub filter: Option<MessageFilter>,
    handler: MessageFn,
}

impl CommandHandler {
    pub fn new(command: &str, handler: impl Fn(&Client, &Message) -> HandlerResult + Send + Sync + 'static) -> CommandHandler {
        CommandHandler {
            options: MatchingOptions::default(),
            triggers: vec!["/".to_string()],
            command: command.to_lowercase(),
            filter: None,
            handler: Box::new(handler),
        }
    }

    pub fn with_filter(mut self, filter: MessageFilter) -> Self {
        self.filter = Some(filter);
        self
    }

    pub fn with_triggers(mut self, triggers: &[&str]) -> Self {
        self.triggers = triggers.iter().map(|s| s.to_string()).collect();
        self
    }
}

impl Handler for CommandHandler {
    fn check_update(&self, client: &Client, update: &Update) -> bool {
        let msg = match update.message() {
            Some(m) => m,
            None => return false,
        };
        if !self.options.check(&msg) || !update.is("updateNewMessage") {
            return false;
        }
        if let Some(f) = &self.filter {
            if !f.test(&msg) {
                return false;
            }
        }
        let text = msg.get_text();
        if text.is_empty() {
            return false;
        }
        for trigger in &self.triggers {
            if !text.starts_with(trigger.as_str()) {
                continue;
            }
            let parsed = match parse_command(text, &[trigger.as_str()]) {
                Some(p) => p,
                None => continue,
            };
            if parsed.name.to_lowercase() != self.command {
                continue;
            }
            if !parsed.mention.is_empty() {
                let me = match client.me() {
                    Some(me) => me,
                    None => return false,
                };
                let uname = me.username().to_lowercase();
                if uname.is_empty() || uname != parsed.mention.to_lowercase() {
                    return false;
                }
            }
            return true;
        }
        false
    }

    fn handle_update(&self, client: &Client, update: &Update) -> HandlerResult {
        match update.message() {
            Some(m) => (self.handler)(client, &m),
            None => Ok(()),
        }
    }
}

/// Handles new messages (optionally filtered).
pub struct MessageHandler {
    pub options: MatchingOptions,
    pub filter: Option<MessageFilter>,
    handler: MessageFn,
}

impl MessageHandler {
    pub fn new(
        handler: impl Fn(&Client, &Message) -> HandlerResult + Send + Sync + 'static,
        filter: Option<MessageFilter>,
    ) -> MessageHandler {
        MessageHandler { options: MatchingOptions::default(), filter, handler: Box::new(handler) }
    }
}

impl Handler for MessageHandler {
    fn check_update(&self, _client: &Client, update: &Update) -> bool {
        if !update.is("updateNewMessage") {
            return false;
        }
        match update.message() {
            Some(m) => self.options.check(&m) && self.filter.as_ref().map_or(true, |f| f.test(&m)),
            None => false,
        }
    }

    fn handle_update(&self, client: &Client, update: &Update) -> HandlerResult {
        match update.message() {
            Some(m) => (self.handler)(client, &m),
            None => Ok(()),
        }
    }
}

/// Handles pressed inline buttons (`updateNewCallbackQuery`).
///
/// With an `action`, only queries whose data is that action (or starts with
/// `action:`, as produced by [`pack_callback`](crate::pack_callback)) match.
pub struct CallbackHandler {
    pub action: Option<String>,
    pub filter: Option<CallbackFilter>,
    handler: CallbackFn,
}

impl CallbackHandler {
    pub fn new(
        action: Option<&str>,
        handler: impl Fn(&Client, &CallbackQuery) -> HandlerResult + Send + Sync + 'static,
        filter: Option<CallbackFilter>,
    ) -> CallbackHandler {
        CallbackHandler {
            action: action.filter(|a| !a.is_empty()).map(str::to_string),
            filter,
            handler: Box::new(handler),
        }
    }
}

impl Handler for CallbackHandler {
    fn check_update(&self, _client: &Client, update: &Update) -> bool {
        let q = match update.callback_query() {
            Some(q) => q,
            None => return false,
        };
        if let Some(action) = &self.action {
            let data = q.data_string();
            let head = data.split(':').next().unwrap_or("");
            if head != action {
                return false;
            }
        }
        self.filter.as_ref().map_or(true, |f| f.test(&q))
    }

    fn handle_update(&self, client: &Client, update: &Update) -> HandlerResult {
        match update.callback_query() {
            Some(q) => (self.handler)(client, &q),
            None => Ok(()),
        }
    }
}

/// Handles any update, optionally restricted to one `@type`.
pub struct UpdateHandler {
    /// Required `@type` (e.g. `"updateChatMember"`); empty = every update.
    pub type_name: String,
    pub filter: Option<Arc<dyn Fn(&Update) -> bool + Send + Sync>>,
    handler: UpdateFn,
}

impl UpdateHandler {
    pub fn new(
        type_name: &str,
        handler: impl Fn(&Client, &Update) -> HandlerResult + Send + Sync + 'static,
        filter: Option<Arc<dyn Fn(&Update) -> bool + Send + Sync>>,
    ) -> UpdateHandler {
        UpdateHandler { type_name: type_name.to_string(), filter, handler: Box::new(handler) }
    }
}

impl Handler for UpdateHandler {
    fn check_update(&self, _client: &Client, update: &Update) -> bool {
        (self.type_name.is_empty() || update.is(&self.type_name)) && self.filter.as_ref().map_or(true, |f| f(update))
    }

    fn handle_update(&self, client: &Client, update: &Update) -> HandlerResult {
        (self.handler)(client, update)
    }
}

/// A named bundle of handlers that can be loaded into a client.
pub struct Plugin {
    pub name: String,
    pub handlers: Vec<(Arc<dyn Handler>, i32)>,
}

impl Plugin {
    pub fn new(name: &str) -> Plugin {
        Plugin { name: name.to_string(), handlers: Vec::new() }
    }

    pub fn add(mut self, handler: impl Handler + 'static, group: i32) -> Plugin {
        self.handlers.push((Arc::new(handler), group));
        self
    }

    pub fn on_message(
        self,
        handler: impl Fn(&Client, &Message) -> HandlerResult + Send + Sync + 'static,
        filter: Option<MessageFilter>,
        group: i32,
    ) -> Plugin {
        self.add(MessageHandler::new(handler, filter), group)
    }

    pub fn on_command(
        self,
        command: &str,
        handler: impl Fn(&Client, &Message) -> HandlerResult + Send + Sync + 'static,
        group: i32,
    ) -> Plugin {
        self.add(CommandHandler::new(command, handler), group)
    }
}
