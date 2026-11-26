//! Convenience methods on [`Client`] for the TDLib calls bots use most.
//! Anything not covered here is one [`Client::invoke`] away.

use std::time::{Duration, Instant};

use crate::builders::{chat_sender, file_from_id, user_sender};
use crate::callback_pack::{pack_callback_signed, unpack_callback_signed};
use crate::client::Client;
use crate::error::{Error, Result};
use crate::formatting::Mode;
use crate::json::Value;
use crate::types::{Chat, FormattedText, Message, User};

/// Options for sending messages. All fields are optional.
#[derive(Clone, Debug, Default)]
pub struct SendOpts {
    /// `"html"`, `"markdown"`, `"markdownv2"`; empty = the client default.
    pub parse_mode: String,
    /// Pre-parsed entities (`textEntity` objects); disables parse mode.
    pub entities: Option<Vec<Value>>,
    pub disable_web_page_preview: bool,
    pub disable_notification: bool,
    pub protect_content: bool,
    pub allow_paid_broadcast: bool,
    pub from_background: bool,
    pub effect_id: i64,
    /// Reply to this message id (0 = none).
    pub reply_to_message_id: i64,
    /// Full `reply_to` object; overrides `reply_to_message_id`.
    pub reply_to: Option<Value>,
    /// Inline/reply keyboard (see the `builders` module).
    pub reply_markup: Option<Value>,
    /// Forum topic (`messageTopic*` object, TDLib 1.8.5x+).
    pub topic: Option<Value>,
    /// Legacy forum thread id (older TDLib).
    pub message_thread_id: i64,
    pub scheduling_state: Option<Value>,
    pub has_spoiler: bool,
    pub show_caption_above_media: bool,
    pub clear_draft: bool,
    /// Extra top-level fields merged into the request, e.g. for TDLib features
    /// newer than this crate.
    pub extra: Option<Value>,
}

impl SendOpts {
    pub fn new() -> SendOpts {
        SendOpts::default()
    }
    pub fn parse_mode(mut self, m: &str) -> Self {
        self.parse_mode = m.to_string();
        self
    }
    pub fn reply_to(mut self, message_id: i64) -> Self {
        self.reply_to_message_id = message_id;
        self
    }
    pub fn reply_markup(mut self, markup: Value) -> Self {
        self.reply_markup = Some(markup);
        self
    }
    pub fn silent(mut self) -> Self {
        self.disable_notification = true;
        self
    }
    pub fn no_preview(mut self) -> Self {
        self.disable_web_page_preview = true;
        self
    }

    fn send_options(&self) -> Value {
        let mut o = Value::typed("messageSendOptions")
            .set("disable_notification", self.disable_notification)
            .set("from_background", self.from_background)
            .set("protect_content", self.protect_content)
            .set("allow_paid_broadcast", self.allow_paid_broadcast);
        if self.effect_id != 0 {
            o.insert("effect_id", self.effect_id);
        }
        if let Some(s) = &self.scheduling_state {
            o.insert("scheduling_state", s.clone());
        }
        o
    }

    fn apply_routing(&self, req: &mut Value) {
        if let Some(r) = &self.reply_to {
            req.insert("reply_to", r.clone());
        } else if self.reply_to_message_id > 0 {
            req.insert(
                "reply_to",
                Value::typed("inputMessageReplyToMessage").set("message_id", self.reply_to_message_id),
            );
        }
        if let Some(m) = &self.reply_markup {
            req.insert("reply_markup", m.clone());
        }
        if let Some(t) = &self.topic {
            req.insert("topic_id", t.clone());
        }
        if self.message_thread_id != 0 {
            req.insert("message_thread_id", self.message_thread_id);
        }
        if let Some(x) = &self.extra {
            req.merge(x);
        }
    }
}

/// Options for editing messages.
#[derive(Clone, Debug, Default)]
pub struct EditOpts {
    pub parse_mode: String,
    pub entities: Option<Vec<Value>>,
    pub disable_web_page_preview: bool,
    pub reply_markup: Option<Value>,
    pub show_caption_above_media: bool,
    pub extra: Option<Value>,
}

/// Kinds of media [`Client::send_media`] can send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Photo,
    Video,
    Animation,
    Audio,
    Document,
    Voice,
    VideoNote,
    Sticker,
}

impl MediaKind {
    fn content_type(self) -> &'static str {
        match self {
            MediaKind::Photo => "inputMessagePhoto",
            MediaKind::Video => "inputMessageVideo",
            MediaKind::Animation => "inputMessageAnimation",
            MediaKind::Audio => "inputMessageAudio",
            MediaKind::Document => "inputMessageDocument",
            MediaKind::Voice => "inputMessageVoiceNote",
            MediaKind::VideoNote => "inputMessageVideoNote",
            MediaKind::Sticker => "inputMessageSticker",
        }
    }
    fn file_field(self) -> &'static str {
        match self {
            MediaKind::Photo => "photo",
            MediaKind::Video => "video",
            MediaKind::Animation => "animation",
            MediaKind::Audio => "audio",
            MediaKind::Document => "document",
            MediaKind::Voice => "voice_note",
            MediaKind::VideoNote => "video_note",
            MediaKind::Sticker => "sticker",
        }
    }
    fn has_caption(self) -> bool {
        !matches!(self, MediaKind::VideoNote | MediaKind::Sticker)
    }
}

fn expect_type(method: &str, expected: &str, v: Value) -> Result<Value> {
    if v.type_name() == expected {
        Ok(v)
    } else {
        Err(Error::UnexpectedResponse { method: method.into(), expected: expected.into(), got: v.type_name().into() })
    }
}

impl Client {
    // ---- reading -----------------------------------------------------------------

    pub fn get_me(&self) -> Result<User> {
        expect_type("getMe", "user", self.invoke(Value::typed("getMe"))?).map(User)
    }

    pub fn get_user(&self, user_id: i64) -> Result<User> {
        expect_type("getUser", "user", self.invoke(Value::typed("getUser").set("user_id", user_id))?).map(User)
    }

    pub fn get_chat(&self, chat_id: i64) -> Result<Chat> {
        expect_type("getChat", "chat", self.invoke(Value::typed("getChat").set("chat_id", chat_id))?).map(Chat)
    }

    pub fn get_message(&self, chat_id: i64, message_id: i64) -> Result<Message> {
        let req = Value::typed("getMessage").set("chat_id", chat_id).set("message_id", message_id);
        expect_type("getMessage", "message", self.invoke(req)?).map(Message)
    }

    /// `chatMember` of `user_id` in `chat_id`.
    pub fn get_chat_member(&self, chat_id: i64, user_id: i64) -> Result<Value> {
        let req = Value::typed("getChatMember").set("chat_id", chat_id).set("member_id", user_sender(user_id));
        expect_type("getChatMember", "chatMember", self.invoke(req)?)
    }

    /// Resolve `@username` (with or without `@`) to a chat.
    pub fn resolve_username(&self, username: &str) -> Result<Chat> {
        let req = Value::typed("searchPublicChat").set("username", username.trim_start_matches('@'));
        expect_type("searchPublicChat", "chat", self.invoke(req)?).map(Chat)
    }

    pub fn get_option(&self, name: &str) -> Result<Value> {
        self.invoke(Value::typed("getOption").set("name", name))
    }

    /// Set a TDLib option; `value` may be a bool, integer, string or `Null` (reset).
    pub fn set_option(&self, name: &str, value: Value) -> Result<()> {
        let ov = match value {
            Value::Bool(b) => Value::typed("optionValueBoolean").set("value", b),
            Value::Int(i) => Value::typed("optionValueInteger").set("value", i),
            Value::Str(s) => Value::typed("optionValueString").set("value", s),
            _ => Value::typed("optionValueEmpty"),
        };
        self.invoke(Value::typed("setOption").set("name", name).set("value", ov)).map(|_| ())
    }

    // ---- text formatting -------------------------------------------------------------

    /// Parse `text` in the given parse mode into a `formattedText`.
    /// Unknown/empty mode returns the text unchanged.
    pub fn parse_text(&self, text: &str, parse_mode: &str) -> Result<FormattedText> {
        let mode = match Mode::parse(parse_mode) {
            Some(Mode::Html) => Value::typed("textParseModeHTML"),
            Some(Mode::Markdown) => Value::typed("textParseModeMarkdown").set("version", 1),
            Some(Mode::MarkdownV2) => Value::typed("textParseModeMarkdown").set("version", 2),
            None => return Ok(FormattedText::plain(text)),
        };
        let req = Value::typed("parseTextEntities").set("text", text).set("parse_mode", mode);
        expect_type("parseTextEntities", "formattedText", self.invoke(req)?).map(FormattedText)
    }

    pub fn html(&self, text: &str) -> Result<FormattedText> {
        self.parse_text(text, "html")
    }
    pub fn markdown(&self, text: &str) -> Result<FormattedText> {
        self.parse_text(text, "markdown")
    }
    pub fn markdown_v2(&self, text: &str) -> Result<FormattedText> {
        self.parse_text(text, "markdownv2")
    }

    /// Build a `formattedText` from text, explicit entities or a parse mode
    /// (falling back to the client's default parse mode).
    pub fn formatted_text(&self, text: &str, entities: Option<&[Value]>, parse_mode: &str) -> Result<FormattedText> {
        if let Some(ents) = entities {
            return Ok(FormattedText(
                Value::typed("formattedText").set("text", text).set("entities", Value::Array(ents.to_vec())),
            ));
        }
        let mode = if parse_mode.is_empty() { self.config().parse_mode.as_str() } else { parse_mode };
        self.parse_text(text, mode)
    }

    // ---- sending -----------------------------------------------------------------------

    /// Send arbitrary `inputMessageContent`; waits until the message is actually sent.
    pub fn send_content(&self, chat_id: i64, content: Value, opts: &SendOpts) -> Result<Message> {
        let mut req = Value::typed("sendMessage")
            .set("chat_id", chat_id)
            .set("options", opts.send_options())
            .set("input_message_content", content);
        opts.apply_routing(&mut req);
        let resp = expect_type("sendMessage", "message", self.invoke(req)?)?;
        self.wait_message(Message(resp))
    }

    /// Send a text message.
    pub fn send_text(&self, chat_id: i64, text: &str, opts: &SendOpts) -> Result<Message> {
        let ft = self.formatted_text(text, opts.entities.as_deref(), &opts.parse_mode)?;
        let mut content = Value::typed("inputMessageText").set("text", ft.0);
        if opts.disable_web_page_preview {
            content.insert("link_preview_options", Value::typed("linkPreviewOptions").set("is_disabled", true));
        }
        if opts.clear_draft {
            content.insert("clear_draft", true);
        }
        self.send_content(chat_id, content, opts)
    }

    /// Send a media message of the given kind. `file` is an input file from the
    /// `builders` module (`file_from_path`, `file_from_remote`, `file_from_id`).
    pub fn send_media(&self, chat_id: i64, kind: MediaKind, file: Value, caption: &str, opts: &SendOpts) -> Result<Message> {
        let content = self.media_content(kind, file, caption, opts)?;
        self.send_content(chat_id, content, opts)
    }

    /// Build the `inputMessage*` content for [`send_media`](Self::send_media) /
    /// [`send_media_group`](Self::send_media_group).
    pub fn media_content(&self, kind: MediaKind, file: Value, caption: &str, opts: &SendOpts) -> Result<Value> {
        let mut c = Value::typed(kind.content_type()).set(kind.file_field(), file);
        if kind.has_caption() {
            let ft = self.formatted_text(caption, opts.entities.as_deref(), &opts.parse_mode)?;
            c.insert("caption", ft.0);
            if matches!(kind, MediaKind::Photo | MediaKind::Video | MediaKind::Animation) {
                c.insert("has_spoiler", opts.has_spoiler);
                c.insert("show_caption_above_media", opts.show_caption_above_media);
            }
        }
        Ok(c)
    }

    pub fn send_photo(&self, chat_id: i64, file: Value, caption: &str, opts: &SendOpts) -> Result<Message> {
        self.send_media(chat_id, MediaKind::Photo, file, caption, opts)
    }
    pub fn send_video(&self, chat_id: i64, file: Value, caption: &str, opts: &SendOpts) -> Result<Message> {
        self.send_media(chat_id, MediaKind::Video, file, caption, opts)
    }
    pub fn send_animation(&self, chat_id: i64, file: Value, caption: &str, opts: &SendOpts) -> Result<Message> {
        self.send_media(chat_id, MediaKind::Animation, file, caption, opts)
    }
    pub fn send_audio(&self, chat_id: i64, file: Value, caption: &str, opts: &SendOpts) -> Result<Message> {
        self.send_media(chat_id, MediaKind::Audio, file, caption, opts)
    }
    pub fn send_document(&self, chat_id: i64, file: Value, caption: &str, opts: &SendOpts) -> Result<Message> {
        self.send_media(chat_id, MediaKind::Document, file, caption, opts)
    }
    pub fn send_voice(&self, chat_id: i64, file: Value, caption: &str, opts: &SendOpts) -> Result<Message> {
        self.send_media(chat_id, MediaKind::Voice, file, caption, opts)
    }
    pub fn send_video_note(&self, chat_id: i64, file: Value, opts: &SendOpts) -> Result<Message> {
        self.send_media(chat_id, MediaKind::VideoNote, file, "", opts)
    }
    pub fn send_sticker(&self, chat_id: i64, file: Value, opts: &SendOpts) -> Result<Message> {
        self.send_media(chat_id, MediaKind::Sticker, file, "", opts)
    }

    /// Send an album from `inputMessage*` contents (see [`media_content`](Self::media_content)).
    pub fn send_media_group(&self, chat_id: i64, contents: Vec<Value>, opts: &SendOpts) -> Result<Vec<Message>> {
        let mut req = Value::typed("sendMessageAlbum")
            .set("chat_id", chat_id)
            .set("options", opts.send_options())
            .set("input_message_contents", Value::Array(contents));
        opts.apply_routing(&mut req);
        req.remove("reply_markup"); // albums take no markup
        let resp = expect_type("sendMessageAlbum", "messages", self.invoke(req)?)?;
        let mut out = Vec::new();
        for m in resp.field("messages").items() {
            out.push(self.wait_message(Message(m.clone()))?);
        }
        Ok(out)
    }

    /// Forward (or copy, with `send_copy`) messages.
    pub fn forward_messages(
        &self,
        chat_id: i64,
        from_chat_id: i64,
        message_ids: &[i64],
        send_copy: bool,
        remove_caption: bool,
    ) -> Result<Vec<Message>> {
        let req = Value::typed("forwardMessages")
            .set("chat_id", chat_id)
            .set("from_chat_id", from_chat_id)
            .set("message_ids", message_ids.to_vec())
            .set("options", SendOpts::default().send_options())
            .set("send_copy", send_copy)
            .set("remove_caption", remove_caption);
        let resp = expect_type("forwardMessages", "messages", self.invoke(req)?)?;
        let mut out = Vec::new();
        for m in resp.field("messages").items() {
            out.push(self.wait_message(Message(m.clone()))?);
        }
        Ok(out)
    }

    /// Reply to `message` with text.
    pub fn reply_text(&self, message: &Message, text: &str, opts: &SendOpts) -> Result<Message> {
        let mut o = opts.clone();
        if o.reply_to_message_id == 0 && o.reply_to.is_none() {
            o.reply_to_message_id = message.id();
        }
        self.send_text(message.chat_id(), text, &o)
    }

    // ---- editing / deleting ------------------------------------------------------------------

    pub fn edit_message_text(&self, chat_id: i64, message_id: i64, text: &str, opts: &EditOpts) -> Result<Message> {
        let ft = self.formatted_text(text, opts.entities.as_deref(), &opts.parse_mode)?;
        let mut content = Value::typed("inputMessageText").set("text", ft.0);
        if opts.disable_web_page_preview {
            content.insert("link_preview_options", Value::typed("linkPreviewOptions").set("is_disabled", true));
        }
        let mut req = Value::typed("editMessageText")
            .set("chat_id", chat_id)
            .set("message_id", message_id)
            .set("input_message_content", content);
        if let Some(m) = &opts.reply_markup {
            req.insert("reply_markup", m.clone());
        }
        if let Some(x) = &opts.extra {
            req.merge(x);
        }
        expect_type("editMessageText", "message", self.invoke(req)?).map(Message)
    }

    pub fn edit_message_caption(&self, chat_id: i64, message_id: i64, caption: &str, opts: &EditOpts) -> Result<Message> {
        let ft = self.formatted_text(caption, opts.entities.as_deref(), &opts.parse_mode)?;
        let mut req = Value::typed("editMessageCaption")
            .set("chat_id", chat_id)
            .set("message_id", message_id)
            .set("caption", ft.0)
            .set("show_caption_above_media", opts.show_caption_above_media);
        if let Some(m) = &opts.reply_markup {
            req.insert("reply_markup", m.clone());
        }
        if let Some(x) = &opts.extra {
            req.merge(x);
        }
        expect_type("editMessageCaption", "message", self.invoke(req)?).map(Message)
    }

    /// Replace (or, with `None`, remove) the reply markup of a message.
    pub fn edit_message_reply_markup(&self, chat_id: i64, message_id: i64, markup: Option<Value>) -> Result<Message> {
        let mut req = Value::typed("editMessageReplyMarkup").set("chat_id", chat_id).set("message_id", message_id);
        if let Some(m) = markup {
            req.insert("reply_markup", m);
        }
        expect_type("editMessageReplyMarkup", "message", self.invoke(req)?).map(Message)
    }

    pub fn delete_messages(&self, chat_id: i64, message_ids: &[i64], revoke: bool) -> Result<()> {
        let req = Value::typed("deleteMessages")
            .set("chat_id", chat_id)
            .set("message_ids", message_ids.to_vec())
            .set("revoke", revoke);
        self.invoke(req).map(|_| ())
    }

    /// Answer a callback query (stops the loading spinner; optional toast/alert/url).
    pub fn answer_callback_query(&self, query_id: i64, text: &str, show_alert: bool, url: &str, cache_time: i32) -> Result<()> {
        let req = Value::typed("answerCallbackQuery")
            .set("callback_query_id", query_id)
            .set("text", text)
            .set("show_alert", show_alert)
            .set("url", url)
            .set("cache_time", cache_time);
        self.invoke(req).map(|_| ())
    }

    // ---- chat actions ---------------------------------------------------------------------------

    /// Send a chat action such as `"chatActionTyping"`.
    pub fn send_chat_action(&self, chat_id: i64, action_type: &str) -> Result<()> {
        let req = Value::typed("sendChatAction").set("chat_id", chat_id).set("action", Value::typed(action_type));
        self.invoke(req).map(|_| ())
    }

    pub fn send_typing(&self, chat_id: i64) -> Result<()> {
        self.send_chat_action(chat_id, "chatActionTyping")
    }

    // ---- moderation -------------------------------------------------------------------------------

    pub fn pin_chat_message(&self, chat_id: i64, message_id: i64, disable_notification: bool, only_for_self: bool) -> Result<()> {
        let req = Value::typed("pinChatMessage")
            .set("chat_id", chat_id)
            .set("message_id", message_id)
            .set("disable_notification", disable_notification)
            .set("only_for_self", only_for_self);
        self.invoke(req).map(|_| ())
    }

    pub fn unpin_chat_message(&self, chat_id: i64, message_id: i64) -> Result<()> {
        self.invoke(Value::typed("unpinChatMessage").set("chat_id", chat_id).set("message_id", message_id)).map(|_| ())
    }

    pub fn set_chat_member_status(&self, chat_id: i64, member: Value, status: Value) -> Result<()> {
        let req = Value::typed("setChatMemberStatus").set("chat_id", chat_id).set("member_id", member).set("status", status);
        self.invoke(req).map(|_| ())
    }

    /// Ban a user until the Unix time `until` (0 = forever).
    pub fn ban_user(&self, chat_id: i64, user_id: i64, until: i64, revoke_messages: bool) -> Result<()> {
        let req = Value::typed("banChatMember")
            .set("chat_id", chat_id)
            .set("member_id", user_sender(user_id))
            .set("banned_until_date", until)
            .set("revoke_messages", revoke_messages);
        self.invoke(req).map(|_| ())
    }

    /// Lift a ban (the user may rejoin).
    pub fn unban_user(&self, chat_id: i64, user_id: i64) -> Result<()> {
        self.set_chat_member_status(chat_id, user_sender(user_id), Value::typed("chatMemberStatusLeft"))
    }

    /// Remove a user from the chat without a lasting ban.
    pub fn kick_user(&self, chat_id: i64, user_id: i64) -> Result<()> {
        self.unban_user(chat_id, user_id)
    }

    /// Restrict a user with a `chatPermissions` object until Unix time `until`.
    pub fn restrict_user(&self, chat_id: i64, user_id: i64, permissions: Value, until: i64) -> Result<()> {
        let status = Value::typed("chatMemberStatusRestricted")
            .set("is_member", true)
            .set("restricted_until_date", until)
            .set("permissions", permissions);
        self.set_chat_member_status(chat_id, user_sender(user_id), status)
    }

    /// Act on a chat as a sender (`messageSenderChat`) instead of a user.
    pub fn chat_sender(&self, chat_id: i64) -> Value {
        chat_sender(chat_id)
    }

    pub fn leave_chat(&self, chat_id: i64) -> Result<()> {
        self.invoke(Value::typed("leaveChat").set("chat_id", chat_id)).map(|_| ())
    }

    // ---- files ------------------------------------------------------------------------------------------

    /// Download a file by TDLib file id. With `synchronous` the call returns when
    /// the download has finished. Returns the `file` object (see `local.path`).
    pub fn download_file(&self, file_id: i64, priority: i32, offset: i64, limit: i64, synchronous: bool) -> Result<Value> {
        let req = Value::typed("downloadFile")
            .set("file_id", file_id)
            .set("priority", priority.clamp(1, 32))
            .set("offset", offset)
            .set("limit", limit)
            .set("synchronous", synchronous);
        let timeout = if synchronous { Duration::from_secs(3600) } else { self.config().request_timeout };
        expect_type("downloadFile", "file", self.invoke_timeout(req, timeout, true)?)
    }

    /// Download the main media of `message` synchronously.
    pub fn download_media(&self, message: &Message) -> Result<Value> {
        let id = message.media_file_id();
        if id == 0 {
            return Err(Error::other("message has no downloadable media"));
        }
        self.download_file(id, 32, 0, 0, true)
    }

    /// Upload in-memory bytes (staged in a temp file) and return the TDLib `file`.
    ///
    /// TDLib reads files from disk itself, so the bytes are written to the
    /// upload directory, `uploadFile` is called, and the staged copy is removed
    /// once TDLib reports the upload finished (or after 2 minutes).
    pub fn upload_bytes(&self, data: &[u8], file_name: &str) -> Result<Value> {
        if data.is_empty() {
            return Err(Error::other("upload_bytes requires non-empty data"));
        }
        let name = if file_name.is_empty() { "upload.bin" } else { file_name };
        let path = self.stage_upload(name, data)?;
        let result = self.upload_staged(&path, data.len() as i64);
        let _ = std::fs::remove_file(&path);
        result
    }

    fn upload_staged(&self, path: &std::path::Path, size: i64) -> Result<Value> {
        let abs = std::fs::canonicalize(path)?;
        let req = Value::typed("uploadFile")
            .set("file", Value::typed("inputFileLocal").set("path", abs.to_string_lossy().as_ref()))
            .set("priority", 1);
        let file = expect_type("uploadFile", "file", self.invoke(req)?)?;
        let id = file.field("id").int();
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut cur = file;
        while Instant::now() < deadline {
            let remote = cur.field("remote");
            if remote.field("is_uploading_completed").bool()
                || (!remote.field("is_uploading_active").bool() && remote.field("uploaded_size").int() >= size)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
            cur = expect_type("getFile", "file", self.invoke(Value::typed("getFile").set("file_id", id))?)?;
        }
        Ok(cur)
    }

    /// Like [`upload_bytes`](Self::upload_bytes) but returns an `inputFileId`
    /// ready for any input-file field.
    pub fn input_file_bytes(&self, data: &[u8], file_name: &str) -> Result<Value> {
        let f = self.upload_bytes(data, file_name)?;
        Ok(file_from_id(f.field("id").int()))
    }

    fn stage_upload(&self, file_name: &str, data: &[u8]) -> Result<std::path::PathBuf> {
        let cfg = self.config();
        let dir = if !cfg.files_directory.is_empty() {
            std::path::Path::new(&cfg.files_directory).join("file")
        } else if !cfg.upload_dir.is_empty() {
            std::path::PathBuf::from(&cfg.upload_dir)
        } else {
            std::path::PathBuf::from("file")
        };
        let safe = sanitize_file_name(file_name).ok_or_else(|| Error::other(format!("invalid upload file name {file_name:?}")))?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(safe);
        std::fs::write(&path, data)?;
        Ok(path)
    }

    // ---- signed callback data ---------------------------------------------------------------------------

    /// HMAC-signed callback data using `ClientOpts::callback_secret`.
    pub fn pack_callback_signed(&self, action: &str, args: &[&str]) -> Result<String> {
        pack_callback_signed(&self.config().callback_secret, action, args)
    }

    /// Verify and unpack data produced by [`pack_callback_signed`](Self::pack_callback_signed).
    pub fn unpack_callback_signed(&self, data: &str) -> Result<(String, Vec<String>)> {
        unpack_callback_signed(&self.config().callback_secret, data)
    }
}

/// Reduce a user-supplied name to a bare file name (no directories, no `..`).
pub(crate) fn sanitize_file_name(name: &str) -> Option<String> {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let last = last.trim();
    if last.is_empty() || last == "." || last == ".." {
        None
    } else {
        Some(last.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(sanitize_file_name("a.txt").as_deref(), Some("a.txt"));
        assert_eq!(sanitize_file_name("../../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(sanitize_file_name("C:\\x\\y.png").as_deref(), Some("y.png"));
        assert_eq!(sanitize_file_name(".."), None);
        assert_eq!(sanitize_file_name("dir/"), None);
        assert_eq!(sanitize_file_name(""), None);
    }
}
