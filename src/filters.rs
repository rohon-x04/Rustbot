//! Composable filters for messages and callback queries.
//!
//! ```ignore
//! use gotdbot::filters::{message as m, Filter};
//! bot.on_message(handler, Some(m::private() & m::incoming() & !m::command()));
//! ```

use std::ops::{BitAnd, BitOr, Not};
use std::sync::Arc;

/// A cloneable predicate over `T`, combinable with `&`, `|` and `!`.
pub struct Filter<T: ?Sized>(Arc<dyn Fn(&T) -> bool + Send + Sync>);

impl<T: ?Sized> Clone for Filter<T> {
    fn clone(&self) -> Self {
        Filter(self.0.clone())
    }
}

impl<T: ?Sized + 'static> Filter<T> {
    pub fn new(f: impl Fn(&T) -> bool + Send + Sync + 'static) -> Filter<T> {
        Filter(Arc::new(f))
    }

    pub fn test(&self, value: &T) -> bool {
        (self.0)(value)
    }

    pub fn and(self, other: Filter<T>) -> Filter<T> {
        Filter::new(move |v| self.test(v) && other.test(v))
    }

    pub fn or(self, other: Filter<T>) -> Filter<T> {
        Filter::new(move |v| self.test(v) || other.test(v))
    }

    pub fn not(self) -> Filter<T> {
        Filter::new(move |v| !self.test(v))
    }

    /// Matches everything.
    pub fn always() -> Filter<T> {
        Filter::new(|_| true)
    }
}

impl<T: ?Sized + 'static> BitAnd for Filter<T> {
    type Output = Filter<T>;
    fn bitand(self, rhs: Filter<T>) -> Filter<T> {
        self.and(rhs)
    }
}
impl<T: ?Sized + 'static> BitOr for Filter<T> {
    type Output = Filter<T>;
    fn bitor(self, rhs: Filter<T>) -> Filter<T> {
        self.or(rhs)
    }
}
impl<T: ?Sized + 'static> Not for Filter<T> {
    type Output = Filter<T>;
    fn not(self) -> Filter<T> {
        Filter::not(self)
    }
}

/// All of the filters must match.
pub fn all_of<T: ?Sized + 'static>(filters: Vec<Filter<T>>) -> Filter<T> {
    Filter::new(move |v| filters.iter().all(|f| f.test(v)))
}

/// At least one filter must match.
pub fn any_of<T: ?Sized + 'static>(filters: Vec<Filter<T>>) -> Filter<T> {
    Filter::new(move |v| filters.iter().any(|f| f.test(v)))
}

/// Filters over [`Message`](crate::types::Message).
pub mod message {
    use super::Filter;
    use crate::command::parse_command;
    use crate::types::Message;

    pub type MessageFilter = Filter<Message>;

    macro_rules! simple {
        ($($(#[$m:meta])* $name:ident => |$arg:ident| $body:expr),* $(,)?) => {
            $( $(#[$m])* pub fn $name() -> MessageFilter { Filter::new(|$arg: &Message| $body) } )*
        };
    }

    simple! {
        /// Messages in private chats.
        private => |m| m.is_private(),
        /// Messages in basic groups.
        group => |m| m.is_group(),
        /// Messages in supergroups and channels (`-100...` ids).
        channel => |m| m.is_supergroup_or_channel(),
        incoming => |m| !m.is_outgoing(),
        outgoing => |m| m.is_outgoing(),
        edited => |m| m.is_edited(),
        /// Starts with a bot-command entity.
        command => |m| m.is_command(),
        text => |m| m.has_text(),
        caption => |m| m.has_caption(),
        photo => |m| m.has_photo(),
        video => |m| m.has_video(),
        animation => |m| m.has_animation(),
        audio => |m| m.has_audio(),
        document => |m| m.has_document(),
        sticker => |m| m.has_sticker(),
        voice => |m| m.has_voice(),
        video_note => |m| m.has_video_note(),
        contact => |m| m.has_contact(),
        location => |m| m.has_location(),
        venue => |m| m.has_venue(),
        poll => |m| m.has_poll(),
        dice => |m| m.has_dice(),
        game => |m| m.has_game(),
        story => |m| m.has_story(),
        invoice => |m| m.has_invoice(),
        paid_media => |m| m.has_paid_media(),
        checklist => |m| m.has_checklist(),
        giveaway => |m| m.has_giveaway(),
        media => |m| m.has_media(),
        media_group => |m| m.is_album(),
        media_spoiler => |m| m.has_spoiler(),
        forwarded => |m| m.is_forwarded(),
        reply => |m| m.is_reply(),
        via_bot => |m| m.is_via_bot(),
        scheduled => |m| m.is_scheduled(),
        service => |m| m.is_service(),
        inline_keyboard => |m| m.reply_markup_type() == "replyMarkupInlineKeyboard",
        reply_keyboard => |m| m.reply_markup_type() == "replyMarkupShowKeyboard",
        new_chat_members => |m| m.is_new_chat_members(),
        left_chat_member => |m| m.is_left_chat_member(),
        pinned_message => |m| m.is_pinned_message(),
        new_chat_title => |m| m.is_new_chat_title(),
        new_chat_photo => |m| m.is_new_chat_photo(),
        delete_chat_photo => |m| m.is_delete_chat_photo(),
    }

    /// Text (or caption) starts with `prefix`.
    pub fn prefix(prefix: &str) -> MessageFilter {
        let p = prefix.to_string();
        Filter::new(move |m: &Message| m.get_text().starts_with(&p))
    }

    /// Text (or caption) ends with `suffix`.
    pub fn suffix(suffix: &str) -> MessageFilter {
        let s = suffix.to_string();
        Filter::new(move |m: &Message| m.get_text().ends_with(&s))
    }

    /// Text (or caption) equals `text`.
    pub fn equal(text: &str) -> MessageFilter {
        let s = text.to_string();
        Filter::new(move |m: &Message| m.get_text() == s)
    }

    /// Text (or caption) contains `needle`.
    pub fn contains(needle: &str) -> MessageFilter {
        let s = needle.to_string();
        Filter::new(move |m: &Message| m.get_text().contains(&s))
    }

    /// Text (or caption) contains `needle`, ignoring case.
    pub fn contains_ci(needle: &str) -> MessageFilter {
        let s = needle.to_lowercase();
        Filter::new(move |m: &Message| m.get_text().to_lowercase().contains(&s))
    }

    /// Matches any of the given command names (prefix `/`).
    pub fn commands(names: &[&str]) -> MessageFilter {
        commands_with_prefix(&["/"], names)
    }

    /// Matches any of the given command names with any of the prefixes.
    pub fn commands_with_prefix(prefixes: &[&str], names: &[&str]) -> MessageFilter {
        let prefixes: Vec<String> = prefixes.iter().map(|s| s.to_string()).collect();
        let names: Vec<String> = names.iter().map(|s| s.to_lowercase()).collect();
        Filter::new(move |m: &Message| {
            let pf: Vec<&str> = prefixes.iter().map(String::as_str).collect();
            match parse_command(m.get_text(), &pf) {
                Some(c) => names.iter().any(|n| *n == c.name.to_lowercase()),
                None => false,
            }
        })
    }

    /// Sent by one of the given user (or chat) ids.
    pub fn user(ids: &[i64]) -> MessageFilter {
        let ids = ids.to_vec();
        Filter::new(move |m: &Message| ids.contains(&m.sender_id()))
    }

    /// Sent in one of the given chats.
    pub fn chat(ids: &[i64]) -> MessageFilter {
        let ids = ids.to_vec();
        Filter::new(move |m: &Message| ids.contains(&m.chat_id()))
    }
}

/// Filters over [`CallbackQuery`](crate::types::CallbackQuery).
pub mod callback {
    use super::Filter;
    use crate::types::CallbackQuery;

    pub type CallbackFilter = Filter<CallbackQuery>;

    pub fn private() -> CallbackFilter {
        Filter::new(|q: &CallbackQuery| q.is_private())
    }

    pub fn group() -> CallbackFilter {
        Filter::new(|q: &CallbackQuery| !q.is_private())
    }

    /// Callback data equals `data`.
    pub fn data_equal(data: &str) -> CallbackFilter {
        let d = data.to_string();
        Filter::new(move |q: &CallbackQuery| q.data_string() == d)
    }

    /// Callback data starts with `prefix`.
    pub fn data_prefix(prefix: &str) -> CallbackFilter {
        let d = prefix.to_string();
        Filter::new(move |q: &CallbackQuery| q.data_string().starts_with(&d))
    }

    /// Pressed by one of the given users.
    pub fn user(ids: &[i64]) -> CallbackFilter {
        let ids = ids.to_vec();
        Filter::new(move |q: &CallbackQuery| ids.contains(&q.sender_user_id()))
    }

    /// Pressed in one of the given chats.
    pub fn chat(ids: &[i64]) -> CallbackFilter {
        let ids = ids.to_vec();
        Filter::new(move |q: &CallbackQuery| ids.contains(&q.chat_id()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::Value;
    use crate::types::Message;

    fn msg(chat: i64, text: &str, outgoing: bool) -> Message {
        Message(
            Value::typed("message")
                .set("chat_id", chat)
                .set("is_outgoing", outgoing)
                .set("content", Value::typed("messageText").set("text", Value::typed("formattedText").set("text", text))),
        )
    }

    #[test]
    fn combinators() {
        let f = message::private() & message::incoming() & !message::prefix("/");
        assert!(f.test(&msg(5, "hi", false)));
        assert!(!f.test(&msg(5, "/hi", false)));
        assert!(!f.test(&msg(-5, "hi", false)));
        assert!(!f.test(&msg(5, "hi", true)));
        let g = message::equal("a") | message::equal("b");
        assert!(g.test(&msg(1, "b", false)) && !g.test(&msg(1, "c", false)));
        assert!(message::commands(&["Start", "help"]).test(&msg(1, "/START@bot x", false)));
        assert!(!message::commands(&["start"]).test(&msg(1, "start", false)));
        assert!(message::user(&[0]).test(&msg(1, "x", false)));
        assert!(message::contains_ci("HEL").test(&msg(1, "say hello", false)));
        assert!(all_of(vec![message::private(), message::text()]).test(&msg(3, "t", false)));
        assert!(!any_of::<Message>(vec![]).test(&msg(3, "t", false)));
    }
}
