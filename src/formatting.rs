//! Convert text plus TDLib text entities back into Markdown / MarkdownV2 / HTML
//! (the inverse of `parseTextEntities`).

use crate::json::Value;

/// Output syntax for [`unparse_entities`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Markdown,
    MarkdownV2,
    Html,
}

impl Mode {
    /// Parse `"html"`, `"markdown"`/`"md"`, `"markdownv2"`/`"mdv2"` (case-insensitive).
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "html" => Some(Mode::Html),
            "markdown" | "md" => Some(Mode::Markdown),
            "markdownv2" | "mdv2" => Some(Mode::MarkdownV2),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Esc {
    Text,
    Url,
    Code,
}

fn escape(text: &str, mode: Mode, kind: Esc) -> String {
    let specials: &[char] = match (mode, kind) {
        (Mode::Html, _) => {
            return text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        }
        (Mode::MarkdownV2, Esc::Text) => {
            &['\\', '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!']
        }
        (Mode::MarkdownV2, Esc::Url) => &['\\', ')'],
        (Mode::MarkdownV2, Esc::Code) | (Mode::Markdown, Esc::Code) => &['\\', '`'],
        (Mode::Markdown, Esc::Text) => &['*', '_', '`', '['],
        (Mode::Markdown, Esc::Url) => &[],
    };
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if specials.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Escape `text` for the given mode.
pub fn escape_text(text: &str, mode: Mode) -> String {
    escape(text, mode, Esc::Text)
}

struct Event {
    index: usize,
    is_start: bool,
    ent: usize,
    level: usize,
}

fn ent_range(e: &Value) -> (usize, usize) {
    let off = e.field("offset").int().max(0) as usize;
    let len = e.field("length").int().max(0) as usize;
    (off, off + len)
}

fn date_format_string(f: &Value) -> String {
    match f.type_name() {
        "dateTimeFormattingTypeRelative" => "r".into(),
        "dateTimeFormattingTypeAbsolute" => {
            let mut s = String::new();
            if f.field("show_day_of_week").bool() {
                s.push('w');
            }
            match f.path(&["date_precision", "@type"]).str() {
                "dateTimePartPrecisionShort" => s.push('d'),
                "dateTimePartPrecisionLong" => s.push('D'),
                _ => {}
            }
            match f.path(&["time_precision", "@type"]).str() {
                "dateTimePartPrecisionShort" => s.push('t'),
                "dateTimePartPrecisionLong" => s.push('T'),
                _ => {}
            }
            s
        }
        _ => String::new(),
    }
}

fn tag(ent: &Value, is_start: bool, mode: Mode) -> String {
    let ty = ent.field("type");
    let html = mode == Mode::Html;
    let v2 = mode == Mode::MarkdownV2;
    let s = |open: &str, close: &str| (if is_start { open } else { close }).to_string();
    match ty.type_name() {
        "textEntityTypeBold" => {
            if html { s("<b>", "</b>") } else { "*".into() }
        }
        "textEntityTypeItalic" => {
            if html { s("<i>", "</i>") } else { s("_\r", "\r_") }
        }
        "textEntityTypeUnderline" => {
            if html {
                s("<u>", "</u>")
            } else if v2 {
                s("__\r", "\r__")
            } else {
                String::new()
            }
        }
        "textEntityTypeStrikethrough" => {
            if html {
                s("<s>", "</s>")
            } else if v2 {
                "~".into()
            } else {
                String::new()
            }
        }
        "textEntityTypeSpoiler" => {
            if html {
                s("<tg-spoiler>", "</tg-spoiler>")
            } else if v2 {
                "||".into()
            } else {
                String::new()
            }
        }
        "textEntityTypeCode" => {
            if html { s("<code>", "</code>") } else { "`".into() }
        }
        "textEntityTypePre" => {
            if html { s("<pre>", "</pre>") } else { "```\n".into() }
        }
        "textEntityTypePreCode" => {
            let lang = ty.field("language").str();
            if html {
                s(&format!("<pre><code class=\"language-{lang}\">"), "</code></pre>")
            } else if is_start {
                format!("```{lang}\n")
            } else {
                "```\n".into()
            }
        }
        "textEntityTypeTextUrl" => {
            let url = ty.field("url").str();
            if html {
                s(&format!("<a href=\"{url}\">"), "</a>")
            } else if is_start {
                "[".into()
            } else {
                format!("]({})", escape(url, mode, Esc::Url))
            }
        }
        "textEntityTypeMentionName" => {
            let id = ty.field("user_id").int();
            if html {
                s(&format!("<a href=\"tg://user?id={id}\">"), "</a>")
            } else if is_start {
                "[".into()
            } else {
                format!("](tg://user?id={id})")
            }
        }
        "textEntityTypeCustomEmoji" => {
            let id = ty.field("custom_emoji_id").int();
            if html {
                s(&format!("<tg-emoji emoji-id=\"{id}\">"), "</tg-emoji>")
            } else if v2 {
                if is_start { "![".into() } else { format!("](tg://emoji?id={id})") }
            } else {
                String::new()
            }
        }
        "textEntityTypeBlockQuote" => {
            if html {
                s("<blockquote>", "</blockquote>")
            } else if v2 {
                s(">", "")
            } else {
                String::new()
            }
        }
        "textEntityTypeExpandableBlockQuote" => {
            if html {
                s("<blockquote expandable>", "</blockquote>")
            } else if v2 {
                s("**>", "||")
            } else {
                String::new()
            }
        }
        "textEntityTypeDateTime" => {
            let unix = ty.field("unix_time").int();
            let fmt = date_format_string(ty.field("formatting_type"));
            if html {
                if is_start {
                    if fmt.is_empty() {
                        format!("<tg-time unix=\"{unix}\">")
                    } else {
                        format!("<tg-time unix=\"{unix}\" format=\"{fmt}\">")
                    }
                } else {
                    "</tg-time>".into()
                }
            } else if v2 {
                if is_start {
                    "![".into()
                } else if fmt.is_empty() {
                    format!("](tg://time?unix={unix})")
                } else {
                    format!("](tg://time?unix={unix}&format={})", escape(&fmt, mode, Esc::Url))
                }
            } else {
                String::new()
            }
        }
        // Implicit entities (URLs, mentions, hashtags, ...) carry no markup of their own.
        _ => String::new(),
    }
}

/// Render `text` with its `entities` (TDLib `textEntity` objects) in `mode`.
pub fn unparse_entities(text: &str, entities: &[Value], mode: Mode) -> String {
    if entities.is_empty() {
        return escape(text, mode, Esc::Text);
    }
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let n = utf16.len();

    let mut events = Vec::with_capacity(entities.len() * 2);
    for (i, e) in entities.iter().enumerate() {
        let (start, end) = ent_range(e);
        events.push(Event { index: start, is_start: true, ent: i, level: i });
        events.push(Event { index: end, is_start: false, ent: i, level: i });
    }
    // Same index: closings first; openings in entity order, closings in reverse order.
    events.sort_by(|a, b| {
        a.index
            .cmp(&b.index)
            .then_with(|| a.is_start.cmp(&b.is_start))
            .then_with(|| if a.is_start { a.level.cmp(&b.level) } else { b.level.cmp(&a.level) })
    });

    let slice = |from: usize, to: usize| String::from_utf16_lossy(&utf16[from.min(n)..to.min(n)]);

    let mut out = String::new();
    let mut last = 0usize;
    for ev in &events {
        if ev.index > last {
            if last < n {
                let end = ev.index.min(n);
                let mut chunk = slice(last, end);
                let (mut in_code, mut raw, mut in_quote) = (false, false, false);
                for e in entities {
                    let (s, en) = ent_range(e);
                    if !(s <= last && en >= end) {
                        continue;
                    }
                    match e.path(&["type", "@type"]).str() {
                        "textEntityTypeCode" | "textEntityTypePre" | "textEntityTypePreCode" => in_code = true,
                        "textEntityTypeUrl"
                        | "textEntityTypeEmailAddress"
                        | "textEntityTypePhoneNumber"
                        | "textEntityTypeHashtag"
                        | "textEntityTypeCashtag"
                        | "textEntityTypeBankCardNumber"
                        | "textEntityTypeBotCommand"
                        | "textEntityTypeMention" => raw = true,
                        "textEntityTypeBlockQuote" | "textEntityTypeExpandableBlockQuote" => in_quote = true,
                        _ => {}
                    }
                }
                if raw {
                    // implicit entity text is emitted verbatim
                } else if in_code && mode != Mode::Html {
                    chunk = escape(&chunk, mode, Esc::Code);
                } else {
                    chunk = escape(&chunk, mode, Esc::Text);
                }
                if in_quote && mode == Mode::MarkdownV2 {
                    chunk = chunk.replace('\n', "\n>");
                }
                out.push_str(&chunk);
            }
            last = ev.index;
        }
        out.push_str(&tag(&entities[ev.ent], ev.is_start, mode));
    }
    if last < n {
        out.push_str(&escape(&slice(last, n), mode, Esc::Text));
    }
    if mode != Mode::Html {
        // "\r" marks the inside edge of `_`/`__` so adjacent markers never fuse.
        for pat in ["_\r", "\r_", "__\r", "\r__"] {
            out = out.replace(pat, pat.trim_matches('\r'));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ent(off: i64, len: i64, ty: Value) -> Value {
        Value::typed("textEntity").set("offset", off).set("length", len).set("type", ty)
    }

    #[test]
    fn html_and_md() {
        let ents = vec![
            ent(0, 5, Value::typed("textEntityTypeBold")),
            ent(6, 5, Value::typed("textEntityTypeItalic")),
            ent(12, 4, Value::typed("textEntityTypeTextUrl").set("url", "https://x.io")),
        ];
        let text = "Hello world <3 a&b";
        // "Hello"=bold, "world"=italic, offset 12 len 4 = "<3 a"
        assert_eq!(
            unparse_entities(text, &ents, Mode::Html),
            "<b>Hello</b> <i>world</i> <a href=\"https://x.io\">&lt;3 a</a>&amp;b"
        );
        assert_eq!(unparse_entities(text, &ents, Mode::Markdown), "*Hello* _world_ [<3 a](https://x.io)&b");
    }

    #[test]
    fn utf16_offsets_and_escaping() {
        // the emoji is 2 UTF-16 units; bold covers "ab"
        let ents = vec![ent(2, 2, Value::typed("textEntityTypeBold"))];
        assert_eq!(unparse_entities("😀ab.c", &ents, Mode::MarkdownV2), "😀*ab*\\.c");
        assert_eq!(unparse_entities("a_b", &[], Mode::Markdown), "a\\_b");
    }

    #[test]
    fn code_is_not_over_escaped() {
        let ents = vec![ent(0, 3, Value::typed("textEntityTypeCode"))];
        assert_eq!(unparse_entities("a_b", &ents, Mode::MarkdownV2), "`a_b`");
    }

    #[test]
    fn nested_ordering() {
        let ents = vec![
            ent(0, 4, Value::typed("textEntityTypeBold")),
            ent(0, 4, Value::typed("textEntityTypeItalic")),
        ];
        assert_eq!(unparse_entities("abcd", &ents, Mode::Html), "<b><i>abcd</i></b>");
    }
}
