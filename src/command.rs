//! Command-line style parsing of message text (`/start@MyBot "two words" arg`).

/// A parsed command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedCommand {
    pub prefix: String,
    pub name: String,
    /// The `@botname` part, without the `@`.
    pub mention: String,
    /// Arguments; single or double quotes group words, backslash escapes inside quotes.
    pub args: Vec<String>,
}

/// Parse `text` as a command using the given prefixes (default `"/"`).
/// An empty-string prefix matches text without any prefix.
pub fn parse_command(text: &str, prefixes: &[&str]) -> Option<ParsedCommand> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let default = ["/"];
    let prefixes: &[&str] = if prefixes.is_empty() { &default } else { prefixes };

    let mut matched: Option<&str> = None;
    let mut has_empty = false;
    for p in prefixes {
        if p.is_empty() {
            has_empty = true;
            continue;
        }
        if text.starts_with(p) {
            matched = Some(p);
            break;
        }
    }
    let prefix = match matched {
        Some(p) => p,
        None if has_empty => "",
        None => return None,
    };

    let rest = text[prefix.len()..].trim();
    if rest.is_empty() {
        return None;
    }
    let (cmd_part, arg_text) = match rest.find(' ') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };
    let cmd_part = cmd_part.trim();
    if cmd_part.is_empty() {
        return None;
    }
    let (name, mention) = match cmd_part.find('@') {
        Some(i) => (&cmd_part[..i], &cmd_part[i + 1..]),
        None => (cmd_part, ""),
    };
    if name.is_empty() {
        return None;
    }
    Some(ParsedCommand {
        prefix: prefix.to_string(),
        name: name.to_string(),
        mention: mention.to_string(),
        args: split_command_args(arg_text.trim()),
    })
}

/// Split an argument string, honouring quotes and backslash escapes inside quotes.
pub fn split_command_args(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut buf = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for r in s.chars() {
        if escaped {
            buf.push(r);
            escaped = false;
        } else if r == '\\' && quote.is_some() {
            escaped = true;
        } else if let Some(q) = quote {
            if r == q {
                quote = None;
            } else {
                buf.push(r);
            }
        } else if r == '"' || r == '\'' {
            quote = Some(r);
        } else if r.is_whitespace() {
            if !buf.is_empty() {
                args.push(std::mem::take(&mut buf));
            }
        } else {
            buf.push(r);
        }
    }
    if !buf.is_empty() {
        args.push(buf);
    }
    args
}

/// True when `text` parses as a command.
pub fn is_command_text(text: &str, prefixes: &[&str]) -> bool {
    parse_command(text, prefixes).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        let c = parse_command("/Ban@MyBot 123 \"spam bot\" 'it\\'s'", &[]).unwrap();
        assert_eq!(c.name, "Ban");
        assert_eq!(c.mention, "MyBot");
        assert_eq!(c.args, vec!["123", "spam bot", "it's"]);
        assert!(parse_command("hello", &[]).is_none());
        assert!(parse_command("/", &[]).is_none());
        assert!(parse_command("/@bot", &[]).is_none());
        assert_eq!(parse_command("!ping x", &["/", "!"]).unwrap().prefix, "!");
        assert_eq!(parse_command("ping", &[""]).unwrap().name, "ping");
    }
}
