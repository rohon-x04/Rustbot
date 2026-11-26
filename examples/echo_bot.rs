//! Echo bot: `/start` greets, every other private message is copied back.
//!
//! API_ID=12345 API_HASH=abc BOT_TOKEN=123:ABC cargo run --example echo_bot
//! (libtdjson must be next to the binary or on the loader path; set
//! TDJSON_PATH to point at it explicitly.)

use gotdbot::{builders::*, filters::message as f, Client, ClientOpts, SendOpts};

fn main() -> gotdbot::Result<()> {
    let api_id: i32 = std::env::var("API_ID").expect("API_ID").parse().expect("API_ID must be a number");
    let api_hash = std::env::var("API_HASH").expect("API_HASH");
    let token = std::env::var("BOT_TOKEN").expect("BOT_TOKEN");

    let opts = ClientOpts { library_path: std::env::var("TDJSON_PATH").unwrap_or_default(), ..ClientOpts::default() };
    let bot = Client::new(api_id, &api_hash, &token, opts)?;

    bot.on_command("start", |c, m| {
        let keyboard = inline_keyboard(vec![inline_row(vec![callback_button("Ping", "ping:1"), url_button("Source", "https://github.com/Iotatg/gotdbot")])]);
        c.reply_text(m, "Hello! I am powered by gotdbot (Rust).", &SendOpts::new().reply_markup(keyboard))?;
        Ok(())
    });

    bot.on_callback("ping", |c, q| {
        c.answer_callback_query(q.id(), "pong", false, "", 0)
    }, None);

    // Everything that is a private, non-command message.
    bot.on_message(
        |c, m| {
            c.forward_messages(m.chat_id(), m.chat_id(), &[m.id()], true, false)?;
            Ok(())
        },
        Some(f::private() & !f::command()),
    );

    bot.start()?;
    println!("started as @{}", bot.me().map(|u| u.username().to_string()).unwrap_or_default());
    bot.idle();
    Ok(())
}
