//! Builders for inline keyboards, reply keyboards and input files.

use crate::crypto::base64_encode;
use crate::json::Value;

// ---- files -------------------------------------------------------------------------

/// A local file path.
pub fn file_from_path(path: &str) -> Value {
    Value::typed("inputFileLocal").set("path", path)
}

/// A file already known to TDLib by its integer id.
pub fn file_from_id(id: i64) -> Value {
    Value::typed("inputFileId").set("id", id)
}

/// A file already stored on Telegram servers (remote file id).
pub fn file_from_remote(id: &str) -> Value {
    Value::typed("inputFileRemote").set("id", id)
}

// ---- inline keyboards ------------------------------------------------------------------------

fn button(text: &str, ty: Value) -> Value {
    Value::typed("inlineKeyboardButton").set("text", text).set("type", ty)
}

/// A button that sends `data` (max 64 bytes) back in a callback query.
pub fn callback_button(text: &str, data: &str) -> Value {
    button(text, Value::typed("inlineKeyboardButtonTypeCallback").set("data", base64_encode(data.as_bytes())))
}

pub fn url_button(text: &str, url: &str) -> Value {
    button(text, Value::typed("inlineKeyboardButtonTypeUrl").set("url", url))
}

pub fn web_app_button(text: &str, url: &str) -> Value {
    button(text, Value::typed("inlineKeyboardButtonTypeWebApp").set("url", url))
}

pub fn user_button(text: &str, user_id: i64) -> Value {
    button(text, Value::typed("inlineKeyboardButtonTypeUser").set("user_id", user_id))
}

pub fn copy_text_button(text: &str, copy_text: &str) -> Value {
    button(text, Value::typed("inlineKeyboardButtonTypeCopyText").set("text", copy_text))
}

/// A button that opens the inline-mode chooser with `query` prefilled.
pub fn switch_inline_button(text: &str, query: &str) -> Value {
    button(
        text,
        Value::typed("inlineKeyboardButtonTypeSwitchInline")
            .set("query", query)
            .set("target_chat", Value::typed("targetChatChosen").set(
                "types",
                Value::typed("targetChatTypes")
                    .set("allow_user_chats", true)
                    .set("allow_bot_chats", true)
                    .set("allow_group_chats", true)
                    .set("allow_channel_chats", true),
            )),
    )
}

/// One row of buttons.
pub fn inline_row(buttons: Vec<Value>) -> Value {
    Value::Array(buttons)
}

/// An inline keyboard from rows built with [`inline_row`].
pub fn inline_keyboard(rows: Vec<Value>) -> Value {
    Value::typed("replyMarkupInlineKeyboard").set("rows", Value::Array(rows))
}

// ---- reply keyboards ---------------------------------------------------------------------------

/// A plain text keyboard button.
pub fn keyboard_button(text: &str) -> Value {
    Value::typed("keyboardButton").set("text", text).set("type", Value::typed("keyboardButtonTypeText"))
}

pub fn keyboard_request_contact(text: &str) -> Value {
    Value::typed("keyboardButton").set("text", text).set("type", Value::typed("keyboardButtonTypeRequestPhoneNumber"))
}

pub fn keyboard_request_location(text: &str) -> Value {
    Value::typed("keyboardButton").set("text", text).set("type", Value::typed("keyboardButtonTypeRequestLocation"))
}

/// A custom reply keyboard.
pub fn reply_keyboard(rows: Vec<Vec<Value>>, resize: bool, one_time: bool, placeholder: &str) -> Value {
    Value::typed("replyMarkupShowKeyboard")
        .set("rows", Value::Array(rows.into_iter().map(Value::Array).collect()))
        .set("resize_keyboard", resize)
        .set("one_time", one_time)
        .set("is_persistent", false)
        .set("is_personal", false)
        .set("input_field_placeholder", placeholder)
}

pub fn remove_keyboard(is_personal: bool) -> Value {
    Value::typed("replyMarkupRemoveKeyboard").set("is_personal", is_personal)
}

pub fn force_reply(placeholder: &str, is_personal: bool) -> Value {
    Value::typed("replyMarkupForceReply").set("input_field_placeholder", placeholder).set("is_personal", is_personal)
}

// ---- chat actions / senders -----------------------------------------------------------------------

/// `messageSenderUser` for use as a `member_id`.
pub fn user_sender(user_id: i64) -> Value {
    Value::typed("messageSenderUser").set("user_id", user_id)
}

/// `messageSenderChat` for use as a `member_id`.
pub fn chat_sender(chat_id: i64) -> Value {
    Value::typed("messageSenderChat").set("chat_id", chat_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_json() {
        let kb = inline_keyboard(vec![inline_row(vec![callback_button("Skip", "play:skip"), url_button("Docs", "https://x")])]);
        assert_eq!(kb.path(&["rows"]).items()[0].items().len(), 2);
        assert_eq!(kb.path(&["rows"]).items()[0].items()[0].path(&["type", "data"]).str(), "cGxheTpza2lw");
        assert_eq!(file_from_id(7).to_json(), r#"{"@type":"inputFileId","id":7}"#);
    }
}
