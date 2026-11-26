//! # gotdbot (Rust)
//!
//! A Rust wrapper for [TDLib](https://github.com/tdlib/td), ported from the Go
//! library *gotdbot*. It loads `libtdjson` at runtime (no C compiler, no build
//! script, **zero crate dependencies**) and gives you:
//!
//! * a [`Client`] for bots *and* user accounts (token, phone, QR, 2FA),
//! * handlers in groups, [`filters`] with `&` / `|` / `!` combinators, middleware,
//! * conversations ([`Client::ask`] / [`Client::ask_from`]),
//! * HMAC-signed [callback packing](callback_pack), flood-wait helpers, rate limiting,
//! * entity → Markdown/HTML [formatting](formatting),
//! * a raw escape hatch ([`Client::invoke`]) that reaches every TDLib method.
//!
//! ```no_run
//! use gotdbot::{Client, ClientOpts, SendOpts};
//!
//! fn main() -> gotdbot::Result<()> {
//!     let bot = Client::new(12345, "API_HASH", "123456:BOT_TOKEN", ClientOpts::default())?;
//!
//!     bot.on_command("start", |c, m| {
//!         c.reply_text(m, "Hello! I am powered by gotdbot (Rust).", &SendOpts::default())?;
//!         Ok(())
//!     });
//!
//!     bot.start()?;
//!     bot.idle();
//!     Ok(())
//! }
//! ```

pub mod builders;
pub mod callback_pack;
pub mod client;
pub mod command;
pub mod conversation;
pub mod crypto;
pub mod error;
pub mod filters;
pub mod formatting;
pub mod handlers;
pub mod json;
pub mod logger;
pub mod methods;
pub mod ratelimit;
mod signal;
pub mod tdjson;
pub mod types;

pub use builders::*;
pub use callback_pack::{pack_callback, pack_callback_signed, unpack_callback, unpack_callback_signed};
pub use client::{AutoRetry, Client, ClientManager, ClientOpts, ErrorHandler, Middleware, PanicHandler, VERSION};
pub use command::{is_command_text, parse_command, split_command_args, ParsedCommand};
pub use conversation::WaitMessageOpts;
pub use error::{retry_after_flood_wait, sleep_flood_wait, Error, HandlerResult, Result, TdError};
pub use filters::{all_of, any_of, Filter};
pub use formatting::{escape_text, unparse_entities, Mode};
pub use handlers::{CallbackHandler, CommandHandler, Handler, MatchingOptions, MessageHandler, Plugin, UpdateHandler};
pub use json::Value;
pub use logger::{Level, Logger};
pub use methods::{EditOpts, MediaKind, SendOpts};
pub use ratelimit::rate_limit;
pub use tdjson::{Backend, TdJson};
pub use types::{
    is_member_status, sender_chat_id, sender_user_id, CallbackQuery, Chat, FormattedText, Message, Update, User,
};
