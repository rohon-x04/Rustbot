//! Minimal JSON value, parser and serializer (no external dependencies).
//!
//! TDLib speaks JSON; every request and response in this crate is a [`Value`].
//! Object key order is preserved, and integers are kept as `i64` so Telegram
//! chat ids such as `-1004379943083` never lose precision.

use std::fmt;

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

/// Error returned by [`Value::parse`].
#[derive(Clone, Debug, PartialEq)]
pub struct JsonError {
    pub pos: usize,
    pub msg: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "json error at byte {}: {}", self.pos, self.msg)
    }
}

impl std::error::Error for JsonError {}

static NULL: Value = Value::Null;

impl Value {
    /// An empty JSON object.
    pub fn object() -> Value {
        Value::Object(Vec::new())
    }

    /// A TDLib object with the given `@type`.
    pub fn typed(type_name: &str) -> Value {
        Value::Object(vec![("@type".to_string(), Value::Str(type_name.to_string()))])
    }

    /// Builder: set `key` to `value` (replacing an existing key) and return self.
    pub fn set(mut self, key: &str, value: impl Into<Value>) -> Value {
        self.insert(key, value);
        self
    }

    /// Set `key` to `value`, replacing an existing key. No-op on non-objects.
    pub fn insert(&mut self, key: &str, value: impl Into<Value>) {
        if let Value::Object(items) = self {
            let value = value.into();
            if let Some(slot) = items.iter_mut().find(|(k, _)| k == key) {
                slot.1 = value;
            } else {
                items.push((key.to_string(), value));
            }
        }
    }

    /// Remove `key` from an object, returning the old value.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        if let Value::Object(items) = self {
            if let Some(i) = items.iter().position(|(k, _)| k == key) {
                return Some(items.remove(i).1);
            }
        }
        None
    }

    /// Merge all keys of `other` (an object) into self (an object).
    pub fn merge(&mut self, other: &Value) {
        if let Value::Object(items) = other {
            for (k, v) in items {
                self.insert(k, v.clone());
            }
        }
    }

    /// Look up an object key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Look up an object key, returning `Null` when missing. Handy for chaining:
    /// `v.field("content").field("text").field("text")`.
    pub fn field(&self, key: &str) -> &Value {
        self.get(key).unwrap_or(&NULL)
    }

    /// Array element.
    pub fn at(&self, index: usize) -> Option<&Value> {
        match self {
            Value::Array(a) => a.get(index),
            _ => None,
        }
    }

    /// Follow a path of object keys.
    pub fn path(&self, keys: &[&str]) -> &Value {
        let mut cur = self;
        for k in keys {
            cur = cur.field(k);
        }
        cur
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Integer view. Accepts ints, integral floats and numeric strings
    /// (TDLib encodes some 64-bit ids as strings).
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Float(f) if f.fract() == 0.0 => Some(*f as i64),
            Value::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            Value::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Vec<(String, Value)>> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    /// String value or `""`.
    pub fn str(&self) -> &str {
        self.as_str().unwrap_or("")
    }

    /// Integer value or `0`.
    pub fn int(&self) -> i64 {
        self.as_i64().unwrap_or(0)
    }

    /// Bool value or `false`.
    pub fn bool(&self) -> bool {
        self.as_bool().unwrap_or(false)
    }

    /// Array items or an empty slice.
    pub fn items(&self) -> &[Value] {
        match self {
            Value::Array(a) => a,
            _ => &[],
        }
    }

    /// The TDLib `@type` of an object, or `""`.
    pub fn type_name(&self) -> &str {
        self.field("@type").str()
    }

    /// Parse a JSON document.
    pub fn parse(src: &str) -> Result<Value, JsonError> {
        let mut p = Parser { s: src.as_bytes(), i: 0, depth: 0 };
        p.ws();
        let v = p.value()?;
        p.ws();
        if p.i != p.s.len() {
            return Err(p.err("trailing characters"));
        }
        Ok(v)
    }

    /// Serialize to a compact JSON string.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Int(i) => out.push_str(&i.to_string()),
            Value::Float(f) => {
                if f.is_finite() {
                    let s = f.to_string();
                    out.push_str(&s);
                    if !s.contains(['.', 'e', 'E']) {
                        out.push_str(".0");
                    }
                } else {
                    out.push_str("null");
                }
            }
            Value::Str(s) => write_str(s, out),
            Value::Array(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Value::Object(o) => {
                out.push('{');
                for (i, (k, v)) in o.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_json())
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---- conversions ----------------------------------------------------------

impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::Str(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::Str(s)
    }
}
impl From<&String> for Value {
    fn from(s: &String) -> Value {
        Value::Str(s.clone())
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}
impl From<i32> for Value {
    fn from(i: i32) -> Value {
        Value::Int(i as i64)
    }
}
impl From<i64> for Value {
    fn from(i: i64) -> Value {
        Value::Int(i)
    }
}
impl From<u32> for Value {
    fn from(i: u32) -> Value {
        Value::Int(i as i64)
    }
}
impl From<u64> for Value {
    fn from(i: u64) -> Value {
        Value::Int(i as i64)
    }
}
impl From<usize> for Value {
    fn from(i: usize) -> Value {
        Value::Int(i as i64)
    }
}
impl From<f64> for Value {
    fn from(f: f64) -> Value {
        Value::Float(f)
    }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Value {
        Value::Array(v.into_iter().map(Into::into).collect())
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Value {
        match v {
            Some(v) => v.into(),
            None => Value::Null,
        }
    }
}

// ---- parser ---------------------------------------------------------------

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

const MAX_DEPTH: usize = 512;

impl<'a> Parser<'a> {
    fn err(&self, msg: &str) -> JsonError {
        JsonError { pos: self.i, msg: msg.to_string() }
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn lit(&mut self, word: &str, v: Value) -> Result<Value, JsonError> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err("invalid literal"))
        }
    }

    fn value(&mut self) -> Result<Value, JsonError> {
        match self.peek() {
            None => Err(self.err("unexpected end of input")),
            Some(b'n') => self.lit("null", Value::Null),
            Some(b't') => self.lit("true", Value::Bool(true)),
            Some(b'f') => self.lit("false", Value::Bool(false)),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b'[') => {
                self.enter()?;
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek() == Some(b']') {
                    self.i += 1;
                } else {
                    loop {
                        self.ws();
                        items.push(self.value()?);
                        self.ws();
                        match self.peek() {
                            Some(b',') => self.i += 1,
                            Some(b']') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(self.err("expected ',' or ']'")),
                        }
                    }
                }
                self.depth -= 1;
                Ok(Value::Array(items))
            }
            Some(b'{') => {
                self.enter()?;
                self.i += 1;
                let mut items: Vec<(String, Value)> = Vec::new();
                self.ws();
                if self.peek() == Some(b'}') {
                    self.i += 1;
                } else {
                    loop {
                        self.ws();
                        if self.peek() != Some(b'"') {
                            return Err(self.err("expected string key"));
                        }
                        let k = self.string()?;
                        self.ws();
                        if self.peek() != Some(b':') {
                            return Err(self.err("expected ':'"));
                        }
                        self.i += 1;
                        self.ws();
                        let v = self.value()?;
                        if let Some(slot) = items.iter_mut().find(|(ek, _)| *ek == k) {
                            slot.1 = v;
                        } else {
                            items.push((k, v));
                        }
                        self.ws();
                        match self.peek() {
                            Some(b',') => self.i += 1,
                            Some(b'}') => {
                                self.i += 1;
                                break;
                            }
                            _ => return Err(self.err("expected ',' or '}'")),
                        }
                    }
                }
                self.depth -= 1;
                Ok(Value::Object(items))
            }
            Some(b'-') | Some(b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn enter(&mut self) -> Result<(), JsonError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            Err(self.err("nesting too deep"))
        } else {
            Ok(())
        }
    }

    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.i;
        let mut is_float = false;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        while let Some(c) = self.peek() {
            match c {
                b'0'..=b'9' => self.i += 1,
                b'.' | b'e' | b'E' | b'+' | b'-' => {
                    is_float = true;
                    self.i += 1;
                }
                _ => break,
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| self.err("bad number"))?;
        if !is_float {
            if let Ok(i) = text.parse::<i64>() {
                return Ok(Value::Int(i));
            }
        }
        text.parse::<f64>().map(Value::Float).map_err(|_| self.err("bad number"))
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        if self.i + 4 > self.s.len() {
            return Err(self.err("short \\u escape"));
        }
        let h = std::str::from_utf8(&self.s[self.i..self.i + 4]).map_err(|_| self.err("bad \\u escape"))?;
        let n = u32::from_str_radix(h, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(n)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.i += 1; // opening quote
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.s.get(self.i).ok_or_else(|| self.err("unterminated string"))?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.s.get(self.i).ok_or_else(|| self.err("unterminated escape"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp) {
                                // high surrogate: need a following \uXXXX low surrogate
                                if self.s[self.i..].starts_with(b"\\u") {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if (0xDC00..0xE000).contains(&lo) {
                                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                    } else {
                                        cp = 0xFFFD;
                                    }
                                } else {
                                    cp = 0xFFFD;
                                }
                            } else if (0xDC00..0xE000).contains(&cp) {
                                cp = 0xFFFD;
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return Err(self.err("invalid escape")),
                    }
                }
                c => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| self.err("invalid utf-8 in string"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let src = r#"{"@type":"message","id":123,"chat_id":-1004379943083,"text":"héllo 😀\n","ok":true,"n":null,"f":1.5,"a":[1,2,{"x":"y"}]}"#;
        let v = Value::parse(src).unwrap();
        assert_eq!(v.field("chat_id").int(), -1004379943083);
        assert_eq!(v.field("text").str(), "héllo 😀\n");
        assert_eq!(v.path(&["a"]).items().len(), 3);
        let again = Value::parse(&v.to_json()).unwrap();
        assert_eq!(v, again);
    }

    #[test]
    fn builder_and_errors() {
        let v = Value::typed("getMe").set("x", 1).set("x", 2);
        assert_eq!(v.to_json(), r#"{"@type":"getMe","x":2}"#);
        assert!(Value::parse("{").is_err());
        assert!(Value::parse("[1,]").is_err());
        assert!(Value::parse("1 2").is_err());
        assert_eq!(Value::from("a\"b\\").to_json(), r#""a\"b\\""#);
    }

    #[test]
    fn string_ids() {
        let v = Value::parse(r#"{"id":"9007199254740993"}"#).unwrap();
        assert_eq!(v.field("id").int(), 9007199254740993);
    }
}
