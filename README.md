# gotdbot (Rust)

A Rust port of the Go library **gotdbot**: a TDLib wrapper for Telegram **bots and user accounts**.
It loads `libtdjson` at runtime (`dlopen`/`LoadLibrary`), so there is **no C compiler, no build script and zero crate dependencies**.

## Features
- `Client` for bot tokens, phone numbers, QR login, 2FA, registration
- Handlers in **groups** (`on_command`, `on_message`, `on_callback`, `on_update`, `on_raw_update`), same dispatch rules as the Go library (`ContinueHandlers`, `ContinueGroups`, `StopPropagation`)
- Composable **filters**: `private() & incoming() & !command()`
- **Middleware**, `rate_limit`, error handler, panic handler, `on_connect` / `on_disconnect`, plugins
- **Conversations**: `ask`, `ask_from`, `wait_for`, with cancel filter and timeout
- Request correlation, **auto-retry** (flood wait, chat/message not found), `send_*` calls that wait for the real message id
- HMAC-signed **callback data**, inline/reply keyboard builders
- Entity -> Markdown / MarkdownV2 / HTML (`original_html()`, `unparse_entities`)
- `invoke(Value)` reaches **every** TDLib method (also ones newer than this crate)

## Quick start
```rust
use gotdbot::{Client, ClientOpts, SendOpts};

fn main() -> gotdbot::Result<()> {
    let bot = Client::new(12345, "API_HASH", "123456:BOT_TOKEN", ClientOpts::default())?;
    bot.on_command("start", |c, m| {
        c.reply_text(m, "Hello!", &SendOpts::default())?;
        Ok(())
    });
    bot.start()?;
    bot.idle();
    Ok(())
}
```
You need a built `libtdjson` (`.so` / `.dylib` / `.dll`) on the loader path, or set `ClientOpts::library_path`.
See `examples/` (`echo_bot`, `conversation`).

## Raw calls
```rust
use gotdbot::Value;
let chat = bot.invoke(Value::typed("getChat").set("chat_id", 777000))?;
println!("{}", chat.field("title").str());
```
Typed views (`Message`, `Update`, `CallbackQuery`, `User`, `Chat`) deref to `Value`, so any field is `.field("name")`.

## Differences from the Go version
- TDLib types are **not generated** (about 2,000 of them, and they change every release). The common ones are thin typed views over JSON; everything else goes through `invoke`.
- Handlers run on threads (one per update), not goroutines/async. No tokio needed.
- `on_*` internal auth/connection handling does not swallow updates: your raw handlers also see them.
- Filters have no regex yet (`prefix`, `suffix`, `equal`, `contains`, `commands`, ...).
- QR login prints the link instead of drawing a QR code.

## Tests
`cargo test` runs unit tests plus end-to-end tests of auth, dispatch, conversations, retry and close against an in-process fake TDLib (`tests/client_flow.rs`). Not yet run against a real `libtdjson`.
