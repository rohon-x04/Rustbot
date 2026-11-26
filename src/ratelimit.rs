//! Per-user rate limiting middleware.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::client::Middleware;
use crate::types::Update;

fn user_id(update: &Update) -> i64 {
    match update.kind() {
        "updateNewMessage" => update.message().map(|m| m.sender_id()).unwrap_or(0),
        "updateNewCallbackQuery" => update.0.field("sender_user_id").int(),
        _ => 0,
    }
}

/// Allow at most `n` messages/callbacks per user within `window`; further ones
/// inside the window are dropped silently. Use with
/// [`Client::use_middleware`](crate::Client::use_middleware):
///
/// ```ignore
/// let mw = gotdbot::rate_limit(5, std::time::Duration::from_secs(10));
/// bot.use_middleware(move |c, u, next| mw(c, u, next));
/// ```
pub fn rate_limit(n: usize, window: Duration) -> Middleware {
    if n == 0 || window.is_zero() {
        return std::sync::Arc::new(|_c, _u, next| next());
    }
    let hits: Mutex<HashMap<i64, Vec<Instant>>> = Mutex::new(HashMap::new());
    std::sync::Arc::new(move |_client, update, next| {
        let uid = user_id(update);
        if uid == 0 {
            return next();
        }
        let now = Instant::now();
        {
            let mut map = hits.lock().unwrap();
            let stamps = map.entry(uid).or_default();
            stamps.retain(|t| now.duration_since(*t) < window);
            if stamps.len() >= n {
                return Ok(());
            }
            stamps.push(now);
        }
        next()
    })
}
