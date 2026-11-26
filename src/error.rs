//! Error type shared by the whole crate.
//!
//! Handler control flow (the Go library's `EndGroups`, `ContinueGroups`,
//! `ContinueHandlers` sentinels) is expressed as dedicated variants so a handler
//! can simply `return Err(Error::ContinueGroups)`.

use std::fmt;

use crate::json::JsonError;

/// An error object returned by TDLib (`{"@type":"error","code":..,"message":..}`).
#[derive(Clone, Debug, PartialEq)]
pub struct TdError {
    pub code: i32,
    pub message: String,
}

impl TdError {
    /// Seconds to wait when this is a flood wait (429), else 0.
    pub fn retry_after(&self) -> u64 {
        const PREFIX: &str = "Too Many Requests: retry after ";
        if self.code == 429 {
            if let Some(rest) = self.message.strip_prefix(PREFIX) {
                return rest.trim().parse().unwrap_or(0);
            }
        }
        0
    }
}

impl fmt::Display for TdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TDLib error {}: {}", self.code, self.message)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// TDLib answered with an error object.
    Td(TdError),
    /// No answer from TDLib in time.
    SendTimeout,
    /// The client has been closed.
    Closed,
    /// `ask` / `wait_for` ran out of time.
    ConversationTimeout,
    /// The user's cancellation filter matched during `ask`.
    ConversationCancelled,
    /// The account needs Telegram Premium to continue.
    WaitPremiumPurchase,
    /// Could not load or call libtdjson.
    Library(String),
    Json(String),
    Io(String),
    Auth(String),
    NoFormattedText,
    InvalidUserId,
    InvalidMessageId,
    CallbackTooLong,
    CallbackInvalid,
    CallbackColon,
    CallbackBadHmac,
    NoCallbackSecret,
    /// Handler control flow: stop all remaining handlers/groups (`EndGroups`).
    StopPropagation,
    /// Handler control flow: skip the rest of this group, continue with the next.
    ContinueGroups,
    /// Handler control flow: keep going with the next handler in this group.
    ContinueHandlers,
    /// TDLib answered with a different type than the call promises.
    UnexpectedResponse { method: String, expected: String, got: String },
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
/// What a handler returns.
pub type HandlerResult = Result<()>;

impl Error {
    pub fn as_td(&self) -> Option<&TdError> {
        match self {
            Error::Td(e) => Some(e),
            _ => None,
        }
    }

    /// Seconds to wait if this is a flood-wait error, else 0.
    pub fn flood_wait_seconds(&self) -> u64 {
        self.as_td().map(|e| e.retry_after()).unwrap_or(0)
    }

    pub fn is_flood_wait(&self) -> bool {
        self.flood_wait_seconds() > 0
    }

    /// True for the three handler control-flow variants.
    pub fn is_control(&self) -> bool {
        matches!(self, Error::StopPropagation | Error::ContinueGroups | Error::ContinueHandlers)
    }

    pub fn other(msg: impl Into<String>) -> Error {
        Error::Other(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Td(e) => e.fmt(f),
            Error::SendTimeout => f.write_str("TDLib send timeout"),
            Error::Closed => f.write_str("client is closed"),
            Error::ConversationTimeout => f.write_str("conversation wait timeout"),
            Error::ConversationCancelled => f.write_str("conversation cancelled"),
            Error::WaitPremiumPurchase => f.write_str("account requires Telegram Premium"),
            Error::Library(s) => write!(f, "tdjson library: {s}"),
            Error::Json(s) => write!(f, "json: {s}"),
            Error::Io(s) => write!(f, "io: {s}"),
            Error::Auth(s) => write!(f, "authorization: {s}"),
            Error::NoFormattedText => f.write_str("message does not contain formatted text or caption"),
            Error::InvalidUserId => f.write_str("user id must be greater than zero"),
            Error::InvalidMessageId => f.write_str("message id must be greater than zero"),
            Error::CallbackTooLong => f.write_str("callback data exceeds 64 bytes"),
            Error::CallbackInvalid => f.write_str("callback data is invalid"),
            Error::CallbackColon => f.write_str("callback action and args must not contain ':'"),
            Error::CallbackBadHmac => f.write_str("callback hmac is invalid"),
            Error::NoCallbackSecret => f.write_str("callback secret is empty"),
            Error::StopPropagation => f.write_str("group iteration ended"),
            Error::ContinueGroups => f.write_str("group iteration continued"),
            Error::ContinueHandlers => f.write_str("handler iteration continued"),
            Error::UnexpectedResponse { method, expected, got } => {
                if got.is_empty() {
                    write!(f, "{method} answered with an untyped response, want {expected}")
                } else {
                    write!(f, "{method} answered with {got}, want {expected}")
                }
            }
            Error::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {}

impl From<TdError> for Error {
    fn from(e: TdError) -> Error {
        Error::Td(e)
    }
}
impl From<JsonError> for Error {
    fn from(e: JsonError) -> Error {
        Error::Json(e.to_string())
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e.to_string())
    }
}
impl From<String> for Error {
    fn from(s: String) -> Error {
        Error::Other(s)
    }
}
impl From<&str> for Error {
    fn from(s: &str) -> Error {
        Error::Other(s.to_string())
    }
}

/// Sleep for the flood-wait interval carried by `err`. Returns `true` if it slept.
pub fn sleep_flood_wait(err: &Error) -> bool {
    let s = err.flood_wait_seconds();
    if s == 0 {
        return false;
    }
    std::thread::sleep(std::time::Duration::from_secs(s));
    true
}

/// If `err` is a flood wait, sleep it out and run `f` once more; otherwise
/// return `err` unchanged.
pub fn retry_after_flood_wait<T>(err: Error, f: impl FnOnce() -> Result<T>) -> Result<T> {
    if sleep_flood_wait(&err) {
        f()
    } else {
        Err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flood_wait() {
        let e = Error::Td(TdError { code: 429, message: "Too Many Requests: retry after 7".into() });
        assert!(e.is_flood_wait());
        assert_eq!(e.flood_wait_seconds(), 7);
        let e = Error::Td(TdError { code: 400, message: "Chat not found".into() });
        assert!(!e.is_flood_wait());
    }
}
