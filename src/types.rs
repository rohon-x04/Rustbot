//! Thin typed views over TDLib JSON objects.
//!
//! TDLib has ~2,000 object types and they change with every release. Instead of
//! generating a struct for each, the objects you handle most (`Update`,
//! `Message`, `CallbackQuery`, `User`, `Chat`, `FormattedText`) get wrappers with
//! the accessors a bot needs, and every wrapper derefs to the raw
//! [`Value`] so any other field is one `.field("name")` away.

use std::fmt;
use std::ops::Deref;

use crate::crypto::base64_decode;
use crate::json::Value;

macro_rules! td_wrapper {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name(pub Value);

        impl $name {
            /// Wrap a raw TDLib object.
            pub fn new(v: Value) -> $name {
                $name(v)
            }
            /// The underlying JSON object.
            pub fn raw(&self) -> &Value {
                &self.0
            }
            /// Unwrap into the underlying JSON object.
            pub fn into_raw(self) -> Value {
                self.0
            }
        }
        impl Deref for $name {
            type Target = Value;
            fn deref(&self) -> &Value {
                &self.0
            }
        }
        impl From<Value> for $name {
            fn from(v: Value) -> $name {
                $name(v)
            }
        }
        impl From<$name> for Value {
            fn from(v: $name) -> Value {
                v.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

td_wrapper!(
    /// Any TDLib update (`updateNewMessage`, `updateNewCallbackQuery`, ...).
    Update
);
td_wrapper!(
    /// A TDLib `message` object.
    Message
);
td_wrapper!(
    /// An `updateNewCallbackQuery` update (a pressed inline button).
    CallbackQuery
);
td_wrapper!(
    /// A TDLib `user` object.
    User
);
td_wrapper!(
    /// A TDLib `chat` object.
    Chat
);
td_wrapper!(
    /// A TDLib `formattedText` object (`text` + `entities`).
    FormattedText
);

/// Supergroups and channels have ids that start with `-100`.
pub fn is_supergroup_or_channel_id(id: i64) -> bool {
    id <= -1_000_000_000_000
}

// ---- Update ----------------------------------------------------------------

impl Update {
    /// The TDLib `@type`, e.g. `"updateNewMessage"`.
    pub fn kind(&self) -> &str {
        self.0.type_name()
    }

    /// True when the update has the given `@type`.
    pub fn is(&self, type_name: &str) -> bool {
        self.kind() == type_name
    }

    /// The message carried by `updateNewMessage`, `updateNewBusinessMessage`
    /// or `updateBusinessMessageEdited`.
    pub fn message(&self) -> Option<Message> {
        match self.kind() {
            "updateNewMessage" => self.0.get("message").filter(|m| !m.is_null()).cloned().map(Message),
            "updateNewBusinessMessage" | "updateBusinessMessageEdited" => {
                self.0.path(&["message", "message"]).as_object().map(|_| Message(self.0.path(&["message", "message"]).clone()))
            }
            _ => None,
        }
    }

    /// The update as a callback query, if it is one.
    pub fn callback_query(&self) -> Option<CallbackQuery> {
        if self.is("updateNewCallbackQuery") {
            Some(CallbackQuery(self.0.clone()))
        } else {
            None
        }
    }

    /// The chat this update belongs to, or 0 if it has none. Used to route
    /// updates to conversation waiters.
    pub fn chat_id(&self) -> i64 {
        if let Some(m) = self.message() {
            return m.chat_id();
        }
        let top = self.0.field("chat_id").int();
        if top != 0 {
            return top;
        }
        match self.kind() {
            "updateMessageSendSucceeded" | "updateMessageSendFailed" => self.0.path(&["message", "chat_id"]).int(),
            _ => 0,
        }
    }
}

// ---- Message ---------------------------------------------------------------

macro_rules! content_checks {
    ($($(#[$m:meta])* $fn_name:ident => $ty:literal),* $(,)?) => {
        impl Message {
            $( $(#[$m])* pub fn $fn_name(&self) -> bool { self.content_type() == $ty } )*
        }
    };
}

content_checks! {
    has_text => "messageText",
    has_photo => "messagePhoto",
    has_video => "messageVideo",
    has_animation => "messageAnimation",
    has_audio => "messageAudio",
    has_document => "messageDocument",
    has_sticker => "messageSticker",
    has_voice => "messageVoiceNote",
    has_video_note => "messageVideoNote",
    has_contact => "messageContact",
    has_location => "messageLocation",
    has_venue => "messageVenue",
    has_poll => "messagePoll",
    has_dice => "messageDice",
    has_game => "messageGame",
    has_story => "messageStory",
    has_invoice => "messageInvoice",
    has_paid_media => "messagePaidMedia",
    has_checklist => "messageChecklist",
    has_giveaway => "messageGiveaway",
    /// Service message: someone joined.
    is_new_chat_members => "messageChatAddMembers",
    /// Service message: someone left.
    is_left_chat_member => "messageChatDeleteMember",
    is_pinned_message => "messagePinMessage",
    is_new_chat_title => "messageChatChangeTitle",
    is_new_chat_photo => "messageChatChangePhoto",
    is_delete_chat_photo => "messageChatDeletePhoto",
}

impl Message {
    pub fn id(&self) -> i64 {
        self.0.field("id").int()
    }

    pub fn chat_id(&self) -> i64 {
        self.0.field("chat_id").int()
    }

    pub fn date(&self) -> i64 {
        self.0.field("date").int()
    }

    pub fn edit_date(&self) -> i64 {
        self.0.field("edit_date").int()
    }

    pub fn is_outgoing(&self) -> bool {
        self.0.field("is_outgoing").bool()
    }

    /// The `@type` of the content (`"messageText"`, `"messagePhoto"`, ...).
    pub fn content_type(&self) -> &str {
        self.0.path(&["content", "@type"]).str()
    }

    pub fn content(&self) -> &Value {
        self.0.field("content")
    }

    // -- chat kind (decided from the id alone; see `Chat::` for exact types) --

    pub fn is_private(&self) -> bool {
        self.chat_id() > 0
    }

    /// Basic group (negative id that is not `-100...`).
    pub fn is_group(&self) -> bool {
        let id = self.chat_id();
        id < 0 && !is_supergroup_or_channel_id(id)
    }

    pub fn is_supergroup_or_channel(&self) -> bool {
        is_supergroup_or_channel_id(self.chat_id())
    }

    /// Alias of [`is_supergroup_or_channel`](Self::is_supergroup_or_channel): the
    /// id alone cannot tell a supergroup from a channel.
    pub fn is_channel(&self) -> bool {
        self.is_supergroup_or_channel()
    }

    pub fn is_edited(&self) -> bool {
        self.edit_date() > 0
    }

    // -- text ----------------------------------------------------------------

    /// Text of a `messageText`, else `""`.
    pub fn text(&self) -> &str {
        if self.has_text() {
            self.0.path(&["content", "text", "text"]).str()
        } else {
            ""
        }
    }

    /// Entities of a `messageText`.
    pub fn entities(&self) -> &[Value] {
        if self.has_text() {
            self.0.path(&["content", "text", "entities"]).items()
        } else {
            &[]
        }
    }

    /// Caption of a media message, else `""`.
    pub fn caption(&self) -> &str {
        if self.has_text() {
            ""
        } else {
            self.0.path(&["content", "caption", "text"]).str()
        }
    }

    pub fn caption_entities(&self) -> &[Value] {
        if self.has_text() {
            &[]
        } else {
            self.0.path(&["content", "caption", "entities"]).items()
        }
    }

    pub fn has_caption(&self) -> bool {
        !self.caption().is_empty()
    }

    /// Caption if present, otherwise the text.
    pub fn get_text(&self) -> &str {
        let c = self.caption();
        if c.is_empty() {
            self.text()
        } else {
            c
        }
    }

    /// Caption entities if present, otherwise the text entities.
    pub fn get_entities(&self) -> &[Value] {
        let c = self.caption_entities();
        if c.is_empty() {
            self.entities()
        } else {
            c
        }
    }

    /// The `formattedText` of a text message.
    pub fn formatted_text(&self) -> Option<FormattedText> {
        if self.has_text() {
            Some(FormattedText(self.0.path(&["content", "text"]).clone()))
        } else {
            None
        }
    }

    /// The `formattedText` of a media caption.
    pub fn formatted_caption(&self) -> Option<FormattedText> {
        let c = self.0.path(&["content", "caption"]);
        if !self.has_text() && c.as_object().is_some() {
            Some(FormattedText(c.clone()))
        } else {
            None
        }
    }

    /// True when the text starts with a `/command` entity.
    pub fn is_command(&self) -> bool {
        self.entities()
            .iter()
            .any(|e| e.path(&["type", "@type"]).str() == "textEntityTypeBotCommand" && e.field("offset").int() == 0)
    }

    pub fn has_text_or_caption(&self) -> bool {
        !self.get_text().trim().is_empty()
    }

    /// Command name without prefix or `@bot` part; `""` if not a command.
    pub fn command(&self) -> String {
        crate::command::parse_command(self.get_text(), &["/"]).map(|c| c.name).unwrap_or_default()
    }

    /// The `@bot` part of the command, without `@`.
    pub fn command_mention(&self) -> String {
        crate::command::parse_command(self.get_text(), &["/"]).map(|c| c.mention).unwrap_or_default()
    }

    /// Quote-aware command arguments.
    pub fn command_args_quoted(&self) -> Vec<String> {
        crate::command::parse_command(self.get_text(), &["/"]).map(|c| c.args).unwrap_or_default()
    }

    /// Arguments after the command, whitespace-split and re-joined with single spaces.
    pub fn args(&self) -> String {
        self.args_list().join(" ")
    }

    /// Whitespace-split arguments after the first word.
    pub fn args_list(&self) -> Vec<String> {
        self.text().split_whitespace().skip(1).map(str::to_string).collect()
    }

    /// Argument at `index` or `""`.
    pub fn arg(&self, index: usize) -> String {
        self.args_list().get(index).cloned().unwrap_or_default()
    }

    /// Everything after the command word, trimmed but with interior spacing
    /// preserved (like Python's `text.split(maxsplit=1)[1].strip()`).
    pub fn payload(&self) -> &str {
        let t = self.text();
        let is_ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c');
        let t = t.trim_start_matches(is_ws);
        let after_word = t.trim_start_matches(|c| !is_ws(c));
        after_word.trim_start_matches(is_ws).trim_end_matches(is_ws)
    }

    pub fn starts_with(&self, prefix: &str) -> bool {
        self.get_text().starts_with(prefix)
    }

    pub fn ends_with(&self, suffix: &str) -> bool {
        self.get_text().ends_with(suffix)
    }

    pub fn trimmed_text(&self) -> &str {
        self.get_text().trim()
    }

    /// True when the message carries no text or caption.
    pub fn is_empty_text(&self) -> bool {
        self.get_text().trim().is_empty()
    }

    // -- sender / reply -------------------------------------------------------

    /// User id, or chat id for chat senders, or 0.
    pub fn sender_id(&self) -> i64 {
        let s = self.0.field("sender_id");
        match s.type_name() {
            "messageSenderUser" => s.field("user_id").int(),
            "messageSenderChat" => s.field("chat_id").int(),
            _ => 0,
        }
    }

    /// The user behind the sender; `None` for channel posts / anonymous admins.
    pub fn sender_user_id(&self) -> Option<i64> {
        sender_user_id(self.0.field("sender_id"))
    }

    /// The chat behind the sender, when the sender is a chat.
    pub fn sender_chat_id(&self) -> Option<i64> {
        sender_chat_id(self.0.field("sender_id"))
    }

    /// Id of the message this one replies to, or 0.
    pub fn reply_to_message_id(&self) -> i64 {
        let r = self.0.field("reply_to");
        if r.type_name() == "messageReplyToMessage" {
            r.field("message_id").int()
        } else {
            0
        }
    }

    pub fn is_reply(&self) -> bool {
        self.reply_to_message_id() != 0
    }

    pub fn is_forwarded(&self) -> bool {
        self.0.field("forward_info").as_object().is_some()
    }

    pub fn is_via_bot(&self) -> bool {
        self.0.field("via_bot_user_id").int() != 0
    }

    pub fn is_scheduled(&self) -> bool {
        self.0.field("scheduling_state").as_object().is_some()
    }

    pub fn is_album(&self) -> bool {
        self.0.field("media_album_id").int() != 0
    }

    /// Album id (0 when the message is not part of an album).
    pub fn media_album_id(&self) -> i64 {
        self.0.field("media_album_id").int()
    }

    pub fn is_mentioned(&self) -> bool {
        self.0.field("contains_unread_mention").bool()
    }

    pub fn is_service(&self) -> bool {
        self.content_type().starts_with("message")
            && !matches!(
                self.content_type(),
                "messageText"
                    | "messagePhoto"
                    | "messageVideo"
                    | "messageAnimation"
                    | "messageAudio"
                    | "messageDocument"
                    | "messageSticker"
                    | "messageVoiceNote"
                    | "messageVideoNote"
                    | "messageContact"
                    | "messageLocation"
                    | "messageVenue"
                    | "messagePoll"
                    | "messageDice"
                    | "messageGame"
                    | "messageStory"
                    | "messageInvoice"
                    | "messagePaidMedia"
                    | "messageChecklist"
                    | "messageGiveaway"
                    | "messageGiveawayWinners"
                    | "messageUnsupported"
            )
    }

    /// True for photo/video/animation/audio/document/sticker/voice/video-note.
    pub fn has_media(&self) -> bool {
        self.has_photo()
            || self.has_video()
            || self.has_animation()
            || self.has_audio()
            || self.has_document()
            || self.has_sticker()
            || self.has_voice()
            || self.has_video_note()
    }

    pub fn has_spoiler(&self) -> bool {
        self.0.path(&["content", "has_spoiler"]).bool()
    }

    /// Reply markup kind: `"replyMarkupInlineKeyboard"`, ... or `""`.
    pub fn reply_markup_type(&self) -> &str {
        self.0.path(&["reply_markup", "@type"]).str()
    }

    /// The remote (cloud) file id of the media in this message, or `""`.
    pub fn remote_file_id(&self) -> String {
        self.media_file().path(&["remote", "id"]).str().to_string()
    }

    pub fn remote_unique_file_id(&self) -> String {
        self.media_file().path(&["remote", "unique_id"]).str().to_string()
    }

    /// The TDLib `file` object of the message's main media, or `Null`.
    pub fn media_file(&self) -> &Value {
        let c = self.0.field("content");
        match c.type_name() {
            "messagePhoto" => {
                // biggest size = last
                c.path(&["photo", "sizes"]).items().last().map(|s| s.field("photo")).unwrap_or(c.field("none"))
            }
            "messageVideo" => c.path(&["video", "video"]),
            "messageAnimation" => c.path(&["animation", "animation"]),
            "messageAudio" => c.path(&["audio", "audio"]),
            "messageDocument" => c.path(&["document", "document"]),
            "messageSticker" => c.path(&["sticker", "sticker"]),
            "messageVoiceNote" => c.path(&["voice_note", "voice"]),
            "messageVideoNote" => c.path(&["video_note", "video"]),
            _ => c.field("none"),
        }
    }

    /// Integer id of the TDLib `file` of the main media (for `download_file`), or 0.
    pub fn media_file_id(&self) -> i64 {
        self.media_file().field("id").int()
    }

    /// Original Markdown of the text (or caption).
    pub fn original_md(&self) -> String {
        self.original(crate::formatting::Mode::Markdown)
    }
    /// Original MarkdownV2 of the text (or caption).
    pub fn original_mdv2(&self) -> String {
        self.original(crate::formatting::Mode::MarkdownV2)
    }
    /// Original HTML of the text (or caption).
    pub fn original_html(&self) -> String {
        self.original(crate::formatting::Mode::Html)
    }

    fn original(&self, mode: crate::formatting::Mode) -> String {
        match self.formatted_text().or_else(|| self.formatted_caption()) {
            Some(ft) => crate::formatting::unparse_entities(ft.text(), ft.entities(), mode),
            None => String::new(),
        }
    }
}

/// The user id behind a `MessageSender`, `None` when the sender is a chat.
pub fn sender_user_id(sender: &Value) -> Option<i64> {
    if sender.type_name() == "messageSenderUser" {
        Some(sender.field("user_id").int())
    } else {
        None
    }
}

/// The chat id behind a `MessageSender`, `None` when the sender is a user.
pub fn sender_chat_id(sender: &Value) -> Option<i64> {
    if sender.type_name() == "messageSenderChat" {
        Some(sender.field("chat_id").int())
    } else {
        None
    }
}

/// True for the plain-member status (not admin, not left).
pub fn is_member_status(status: &Value) -> bool {
    status.type_name() == "chatMemberStatusMember"
}

// ---- FormattedText -----------------------------------------------------------

impl FormattedText {
    /// Build plain text without entities.
    pub fn plain(text: &str) -> FormattedText {
        FormattedText(Value::typed("formattedText").set("text", text).set("entities", Vec::<Value>::new()))
    }

    pub fn text(&self) -> &str {
        self.0.field("text").str()
    }

    pub fn entities(&self) -> &[Value] {
        self.0.field("entities").items()
    }
}

// ---- User / Chat -------------------------------------------------------------

impl User {
    pub fn id(&self) -> i64 {
        self.0.field("id").int()
    }
    pub fn first_name(&self) -> &str {
        self.0.field("first_name").str()
    }
    pub fn last_name(&self) -> &str {
        self.0.field("last_name").str()
    }
    /// First active username, or `""`.
    pub fn username(&self) -> &str {
        self.0.path(&["usernames", "active_usernames"]).items().first().map(|v| v.str()).unwrap_or("")
    }
    pub fn is_bot(&self) -> bool {
        self.0.path(&["type", "@type"]).str() == "userTypeBot"
    }
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name(), self.last_name()).trim().to_string()
    }
}

impl Chat {
    pub fn id(&self) -> i64 {
        self.0.field("id").int()
    }
    pub fn title(&self) -> &str {
        self.0.field("title").str()
    }
    pub fn chat_type(&self) -> &str {
        self.0.path(&["type", "@type"]).str()
    }
    pub fn is_basic_group(&self) -> bool {
        self.chat_type() == "chatTypeBasicGroup"
    }
    pub fn is_supergroup(&self) -> bool {
        self.chat_type() == "chatTypeSupergroup"
    }
    /// TDLib reports both supergroups and channels as `chatTypeSupergroup`;
    /// `is_channel_flag` tells them apart.
    pub fn is_channel_flag(&self) -> bool {
        self.is_supergroup() && self.0.path(&["type", "is_channel"]).bool()
    }
    pub fn is_private(&self) -> bool {
        self.chat_type() == "chatTypePrivate"
    }
    pub fn is_secret(&self) -> bool {
        self.chat_type() == "chatTypeSecret"
    }
}

// ---- CallbackQuery -------------------------------------------------------------

impl CallbackQuery {
    /// Query id (needed to answer it).
    pub fn id(&self) -> i64 {
        self.0.field("id").int()
    }
    pub fn sender_user_id(&self) -> i64 {
        self.0.field("sender_user_id").int()
    }
    pub fn chat_id(&self) -> i64 {
        self.0.field("chat_id").int()
    }
    pub fn message_id(&self) -> i64 {
        self.0.field("message_id").int()
    }
    pub fn is_private(&self) -> bool {
        self.chat_id() > 0
    }

    /// Raw callback data bytes (TDLib sends them base64-encoded).
    pub fn data(&self) -> Vec<u8> {
        let p = self.0.field("payload");
        match p.type_name() {
            "callbackQueryPayloadData" | "callbackQueryPayloadDataWithPassword" => {
                base64_decode(p.field("data").str()).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }

    /// Callback data as a string.
    pub fn data_string(&self) -> String {
        String::from_utf8_lossy(&self.data()).into_owned()
    }

    /// Password for `callbackQueryPayloadDataWithPassword`.
    pub fn password(&self) -> &str {
        self.0.path(&["payload", "password"]).str()
    }

    /// Short name of the game for `callbackQueryPayloadGame`.
    pub fn game_short_name(&self) -> &str {
        let p = self.0.field("payload");
        if p.type_name() == "callbackQueryPayloadGame" {
            p.field("game_short_name").str()
        } else {
            ""
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_msg(chat: i64, text: &str) -> Message {
        Message(
            Value::parse(&format!(
                r#"{{"@type":"message","id":5,"chat_id":{chat},"is_outgoing":false,"edit_date":0,
                "sender_id":{{"@type":"messageSenderUser","user_id":42}},
                "content":{{"@type":"messageText","text":{{"@type":"formattedText","text":{t},"entities":[{{"@type":"textEntity","offset":0,"length":6,"type":{{"@type":"textEntityTypeBotCommand"}}}}]}}}}}}"#,
                t = Value::from(text).to_json()
            ))
            .unwrap(),
        )
    }

    #[test]
    fn message_basics() {
        let m = text_msg(-1004379943083, "/gift  10   20");
        assert!(m.is_supergroup_or_channel() && !m.is_group() && !m.is_private());
        assert_eq!(m.sender_id(), 42);
        assert_eq!(m.sender_user_id(), Some(42));
        assert_eq!(m.sender_chat_id(), None);
        assert!(m.is_command());
        assert_eq!(m.args(), "10 20");
        assert_eq!(m.args_list(), vec!["10", "20"]);
        assert_eq!(m.arg(1), "20");
        assert_eq!(m.arg(5), "");
        assert_eq!(m.payload(), "10   20");
        assert_eq!(m.command(), "gift");
        assert!(text_msg(7, "x").is_private());
        assert!(text_msg(-5, "x").is_group());
    }

    #[test]
    fn payload_edge_cases() {
        assert_eq!(text_msg(1, "/welcome welcome to the group  ").payload(), "welcome to the group");
        assert_eq!(text_msg(1, "/start").payload(), "");
        assert_eq!(text_msg(1, "  /start   ").payload(), "");
    }

    #[test]
    fn update_routing() {
        let u = Update(Value::typed("updateNewMessage").set("message", text_msg(9, "hi").0));
        assert_eq!(u.chat_id(), 9);
        assert_eq!(u.message().unwrap().id(), 5);
        let cb = Update(Value::parse(
            r#"{"@type":"updateNewCallbackQuery","id":"77","sender_user_id":3,"chat_id":11,"message_id":4,
                "payload":{"@type":"callbackQueryPayloadData","data":"cGxheTpza2lw"}}"#,
        ).unwrap());
        assert_eq!(cb.chat_id(), 11);
        let q = cb.callback_query().unwrap();
        assert_eq!(q.id(), 77);
        assert_eq!(q.data_string(), "play:skip");
    }

    #[test]
    fn caption_vs_text() {
        let m = Message(Value::parse(
            r#"{"@type":"message","id":1,"chat_id":2,"content":{"@type":"messagePhoto","caption":{"text":"cap","entities":[]},
               "photo":{"sizes":[{"photo":{"id":1,"remote":{"id":"a"}}},{"photo":{"id":2,"remote":{"id":"b","unique_id":"u"}}}]}}}"#,
        ).unwrap());
        assert_eq!(m.get_text(), "cap");
        assert_eq!(m.text(), "");
        assert!(m.has_photo() && m.has_media() && !m.is_service());
        assert_eq!(m.remote_file_id(), "b");
        assert_eq!(m.media_file_id(), 2);
    }
}
