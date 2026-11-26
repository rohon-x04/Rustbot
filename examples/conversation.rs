//! A multi-step conversation with `Client::ask`.
//!
//! API_ID=12345 API_HASH=abc BOT_TOKEN=123:ABC cargo run --example conversation

use gotdbot::{filters::message as f, Client, ClientOpts, Error, SendOpts, WaitMessageOpts};

fn main() -> gotdbot::Result<()> {
    let api_id: i32 = std::env::var("API_ID").expect("API_ID").parse().expect("number");
    let api_hash = std::env::var("API_HASH").expect("API_HASH");
    let token = std::env::var("BOT_TOKEN").expect("BOT_TOKEN");
    let bot = Client::new(api_id, &api_hash, &token, ClientOpts::default())?;

    bot.on_command("rename", |c, m| {
        let o = SendOpts::default();
        c.reply_text(m, "What is your new name? (send /cancel to abort)", &o)?;

        let wait = WaitMessageOpts {
            cancellation_filter: Some(f::commands(&["cancel"])),
            ..WaitMessageOpts::timeout(30)
        };
        // `ask_from` only accepts messages from this user, which matters in groups.
        let user = m.sender_id();
        match c.ask_from(m.chat_id(), user, Some(wait)) {
            Ok(answer) => {
                c.reply_text(&answer, &format!("Nice to meet you, {}!", answer.get_text()), &o)?;
            }
            Err(Error::ConversationTimeout) => {
                c.reply_text(m, "Too slow, try again.", &o)?;
            }
            Err(Error::ConversationCancelled) => {
                c.reply_text(m, "Cancelled.", &o)?;
            }
            Err(e) => return Err(e),
        }
        Ok(())
    });

    bot.start()?;
    bot.idle();
    Ok(())
}
