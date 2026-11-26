//! End-to-end tests of the client against an in-process fake TDLib.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use gotdbot::{
    builders::*, rate_limit, AutoRetry, Backend, Client, ClientOpts, Error, Level, Logger, SendOpts, Value,
    WaitMessageOpts,
};

// ---- fake TDLib ---------------------------------------------------------------------

type Script = Box<dyn FnMut(&Value) -> Vec<Value> + Send>;

struct Mock {
    queue: Mutex<VecDeque<String>>,
    cv: Condvar,
    sent: Mutex<Vec<Value>>,
    script: Mutex<Script>,
}

impl Mock {
    fn new(script: Script) -> Arc<Mock> {
        Arc::new(Mock { queue: Mutex::new(VecDeque::new()), cv: Condvar::new(), sent: Mutex::new(vec![]), script: Mutex::new(script) })
    }

    fn push(&self, mut v: Value) {
        v.insert("@client_id", 1);
        self.queue.lock().unwrap().push_back(v.to_json());
        self.cv.notify_all();
    }

    fn sent_of(&self, ty: &str) -> Vec<Value> {
        self.sent.lock().unwrap().iter().filter(|v| v.type_name() == ty).cloned().collect()
    }
}

impl Backend for Mock {
    fn create_client_id(&self) -> i32 {
        1
    }

    fn send(&self, _client_id: i32, request: &str) {
        let req = Value::parse(request).unwrap();
        self.sent.lock().unwrap().push(req.clone());
        let replies = (self.script.lock().unwrap())(&req);
        for r in replies {
            self.push(r);
        }
    }

    fn receive(&self, timeout: f64) -> Option<String> {
        let mut q = self.queue.lock().unwrap();
        if q.is_empty() {
            q = self.cv.wait_timeout(q, Duration::from_secs_f64(timeout)).unwrap().0;
        }
        q.pop_front()
    }

    fn execute(&self, _request: &str) -> Option<String> {
        None
    }
}

fn reply(req: &Value, mut obj: Value) -> Value {
    if let Some(e) = req.get("@extra") {
        obj.insert("@extra", e.clone());
    }
    obj
}

fn ok(req: &Value) -> Value {
    reply(req, Value::typed("ok"))
}

fn td_error(req: &Value, code: i32, msg: &str) -> Value {
    reply(req, Value::typed("error").set("code", code).set("message", msg))
}

fn auth_state(ty: &str) -> Value {
    Value::typed("updateAuthorizationState").set("authorization_state", Value::typed(ty))
}

fn bot_user() -> Value {
    Value::typed("user")
        .set("id", 99)
        .set("first_name", "My Bot")
        .set("usernames", Value::typed("usernames").set("active_usernames", vec!["MyBot"]))
        .set("type", Value::typed("userTypeBot"))
}

/// The default fake server: authorizes bots and answers the common calls.
fn server(extra: impl FnMut(&Value) -> Option<Vec<Value>> + Send + 'static) -> Script {
    let mut extra = extra;
    let mut next_id = 1000;
    Box::new(move |req| {
        if let Some(r) = extra(req) {
            return r;
        }
        match req.type_name() {
            "getOption" => vec![auth_state("authorizationStateWaitTdlibParameters")],
            "setTdlibParameters" => vec![ok(req), auth_state("authorizationStateWaitPhoneNumber")],
            "checkAuthenticationBotToken" => vec![ok(req), auth_state("authorizationStateReady")],
            "getMe" => vec![reply(req, bot_user())],
            "sendMessage" => {
                next_id += 1000;
                let chat = req.field("chat_id").int();
                let text = req.path(&["input_message_content", "text", "text"]).str().to_string();
                let pending = Value::typed("message")
                    .set("id", next_id)
                    .set("chat_id", chat)
                    .set("sending_state", Value::typed("messageSendingStatePending"))
                    .set("content", Value::typed("messageText").set("text", Value::typed("formattedText").set("text", text.as_str())));
                let sent = Value::typed("message").set("id", next_id + 1).set("chat_id", chat).set(
                    "content",
                    Value::typed("messageText").set("text", Value::typed("formattedText").set("text", text.as_str())),
                );
                vec![
                    reply(req, pending),
                    Value::typed("updateMessageSendSucceeded").set("old_message_id", next_id).set("message", sent),
                ]
            }
            "answerCallbackQuery" | "deleteMessages" | "leaveChat" | "forwardMessages" => vec![ok(req)],
            "close" => vec![ok(req), auth_state("authorizationStateClosed")],
            _ => vec![],
        }
    })
}

fn quiet() -> ClientOpts {
    ClientOpts {
        logger: Arc::new(Logger::new(Level::Off)),
        authorization_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(5),
        log_verbosity_level: -1,
        ..ClientOpts::default()
    }
}

fn boot(script: Script, tweak: impl FnOnce(&mut ClientOpts)) -> (Client, Arc<Mock>) {
    let mock = Mock::new(script);
    let mut opts = quiet();
    opts.backend = Some(mock.clone());
    tweak(&mut opts);
    let client = Client::new(1, "hash", "123456:ABC-def_x", opts).unwrap();
    (client, mock)
}

fn message_update(chat: i64, id: i64, sender: i64, text: &str) -> Value {
    let len = text.split_whitespace().next().filter(|w| w.starts_with('/')).map(|w| w.encode_utf16().count()).unwrap_or(0);
    let entities = if len > 0 {
        vec![Value::typed("textEntity").set("offset", 0).set("length", len).set("type", Value::typed("textEntityTypeBotCommand"))]
    } else {
        vec![]
    };
    let msg = Value::typed("message")
        .set("id", id)
        .set("chat_id", chat)
        .set("is_outgoing", false)
        .set("date", 1)
        .set("edit_date", 0)
        .set("sender_id", Value::typed("messageSenderUser").set("user_id", sender))
        .set(
            "content",
            Value::typed("messageText").set(
                "text",
                Value::typed("formattedText").set("text", text).set("entities", Value::Array(entities)),
            ),
        );
    Value::typed("updateNewMessage").set("message", msg)
}

fn recv<T>(rx: &Receiver<T>) -> T {
    rx.recv_timeout(Duration::from_secs(5)).expect("timed out waiting for handler")
}

// ---- tests -----------------------------------------------------------------------------

#[test]
fn bot_authorizes_and_exposes_me() {
    let (bot, mock) = boot(server(|_| None), |_| {});
    bot.start().unwrap();
    let me = bot.me().unwrap();
    assert_eq!(me.id(), 99);
    assert_eq!(me.username(), "MyBot");
    assert!(me.is_bot());

    let params = &mock.sent_of("setTdlibParameters")[0];
    assert_eq!(params.field("api_id").int(), 1);
    assert_eq!(params.field("api_hash").str(), "hash");
    assert_eq!(params.field("database_directory").str(), "database");
    assert_eq!(mock.sent_of("checkAuthenticationBotToken")[0].field("token").str(), "123456:ABC-def_x");
    bot.close();
}

#[test]
fn auth_failure_is_reported() {
    let script = server(|req| {
        if req.type_name() == "checkAuthenticationBotToken" {
            return Some(vec![td_error(req, 401, "Unauthorized")]);
        }
        None
    });
    let (bot, _mock) = boot(script, |_| {});
    match bot.start() {
        Err(Error::Td(e)) => assert_eq!((e.code, e.message.as_str()), (401, "Unauthorized")),
        other => panic!("expected TDLib error, got {other:?}"),
    }
}

#[test]
fn command_handler_replies_with_reply_to_and_waits_for_send() {
    let (bot, mock) = boot(server(|_| None), |_| {});
    let (tx, rx) = channel();
    bot.on_command("ping", move |c, m| {
        let sent = c.reply_text(m, "pong", &SendOpts::default())?;
        tx.send((sent.id(), sent.text().to_string())).unwrap();
        Ok(())
    });
    bot.start().unwrap();

    mock.push(message_update(7, 55, 42, "/ping"));
    let (id, text) = recv(&rx);
    // the final id from updateMessageSendSucceeded, not the temporary one
    assert_eq!(id, 2001);
    assert_eq!(text, "pong");

    let req = &mock.sent_of("sendMessage")[0];
    assert_eq!(req.field("chat_id").int(), 7);
    assert_eq!(req.path(&["reply_to", "message_id"]).int(), 55);
    assert_eq!(req.path(&["input_message_content", "@type"]).str(), "inputMessageText");
    bot.close();
}

#[test]
fn command_with_bot_mention_only_matches_this_bot() {
    let (bot, mock) = boot(server(|_| None), |_| {});
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    bot.on_command("start", move |_c, _m| {
        h.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    bot.start().unwrap();

    mock.push(message_update(1, 1, 5, "/start@OtherBot"));
    mock.push(message_update(1, 2, 5, "/START@mybot"));
    mock.push(message_update(1, 3, 5, "/starting"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while hits.load(Ordering::SeqCst) < 1 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    bot.close();
}

#[test]
fn handler_groups_and_control_flow() {
    let (bot, mock) = boot(server(|_| None), |_| {});
    let (tx, rx) = channel::<&'static str>();

    let t = tx.clone();
    bot.on_message_group(move |_c, _m| { t.send("g0-a").unwrap(); Err(Error::ContinueHandlers) }, None, 0);
    let t = tx.clone();
    bot.on_message_group(move |_c, _m| { t.send("g0-b").unwrap(); Err(Error::ContinueGroups) }, None, 0);
    let t = tx.clone();
    bot.on_message_group(move |_c, _m| { t.send("g0-never").unwrap(); Ok(()) }, None, 0);
    let t = tx.clone();
    bot.on_message_group(move |_c, _m| { t.send("g1").unwrap(); Ok(()) }, None, 1);
    let t = tx.clone();
    bot.on_message_group(move |_c, _m| { t.send("g2-never").unwrap(); Ok(()) }, None, 2);
    bot.start().unwrap();

    mock.push(message_update(1, 1, 5, "hello"));
    let mut seen = vec![recv(&rx), recv(&rx), recv(&rx)];
    seen.sort();
    assert_eq!(seen, vec!["g0-a", "g0-b", "g1"]);
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "handlers after a plain Ok must not run");
    bot.close();
}

#[test]
fn error_handler_and_panic_handler() {
    let (bot, mock) = boot(server(|_| None), |_| {});
    let (tx, rx) = channel::<String>();
    let t = tx.clone();
    bot.on_error(move |_c, u, e| {
        t.send(format!("error:{}:{}", u.kind(), e)).unwrap();
        Ok(())
    });
    let t = tx.clone();
    bot.on_panic(move |_c, _u, msg| t.send(format!("panic:{msg}")).unwrap());
    bot.on_command("fail", |_c, _m| Err(Error::other("boom")));
    bot.on_command("panic", |_c, _m| panic!("kaboom"));
    bot.start().unwrap();

    mock.push(message_update(1, 1, 5, "/fail"));
    assert_eq!(recv(&rx), "error:updateNewMessage:boom");
    mock.push(message_update(1, 2, 5, "/panic"));
    assert_eq!(recv(&rx), "panic:kaboom");
    bot.close();
}

#[test]
fn conversation_ask_from_with_timeout_and_cancel() {
    let (bot, mock) = boot(server(|_| None), |_| {});
    let (tx, rx) = channel::<String>();
    bot.on_command("rename", move |c, m| {
        let wait = WaitMessageOpts {
            cancellation_filter: Some(gotdbot::filters::message::commands(&["cancel"])),
            ..WaitMessageOpts::timeout(2)
        };
        let r = c.ask_from(m.chat_id(), m.sender_id(), Some(wait));
        tx.send(match r {
            Ok(a) => format!("answer:{}", a.get_text()),
            Err(Error::ConversationTimeout) => "timeout".into(),
            Err(Error::ConversationCancelled) => "cancelled".into(),
            Err(e) => format!("err:{e}"),
        })
        .unwrap();
        Ok(())
    });
    bot.start().unwrap();

    // 1) answer from another user in the same chat is ignored; the right user is accepted
    mock.push(message_update(10, 1, 42, "/rename"));
    std::thread::sleep(Duration::from_millis(300));
    mock.push(message_update(10, 2, 43, "intruder"));
    mock.push(message_update(10, 3, 42, "Alice"));
    assert_eq!(recv(&rx), "answer:Alice");

    // 2) cancellation
    mock.push(message_update(10, 4, 42, "/rename"));
    std::thread::sleep(Duration::from_millis(300));
    mock.push(message_update(10, 5, 42, "/cancel"));
    assert_eq!(recv(&rx), "cancelled");

    // 3) timeout (nobody answers)
    mock.push(message_update(10, 6, 42, "/rename"));
    assert_eq!(recv(&rx), "timeout");

    // invalid user id is rejected up front
    assert_eq!(bot.ask_from(10, 0, None).unwrap_err(), Error::InvalidUserId);
    bot.close();
}

#[test]
fn callback_query_routing_and_answer() {
    let (bot, mock) = boot(server(|_| None), |_| {});
    let (tx, rx) = channel::<(String, i64)>();
    bot.on_callback("play", move |c, q| {
        let (action, args) = gotdbot::unpack_callback(&q.data_string())?;
        c.answer_callback_query(q.id(), "ok", false, "", 0)?;
        tx.send((format!("{action}/{}", args.join(",")), q.sender_user_id())).unwrap();
        Ok(())
    }, None);
    bot.start().unwrap();

    let cb = |data: &str| {
        Value::typed("updateNewCallbackQuery")
            .set("id", "777")
            .set("sender_user_id", 5)
            .set("chat_id", 9)
            .set("message_id", 1)
            .set("payload", Value::typed("callbackQueryPayloadData").set("data", gotdbot::crypto::base64_encode(data.as_bytes())))
    };
    mock.push(cb("other:1"));
    mock.push(cb("play:skip:3"));
    assert_eq!(recv(&rx), ("play/skip,3".to_string(), 5));
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    assert_eq!(mock.sent_of("answerCallbackQuery")[0].field("callback_query_id").int(), 777);
    bot.close();
}

#[test]
fn middleware_rate_limit_and_ordering() {
    let (bot, mock) = boot(server(|_| None), |_| {});
    let mw = rate_limit(2, Duration::from_secs(30));
    bot.use_middleware(move |c, u, next| mw(c, u, next));
    let order = Arc::new(Mutex::new(Vec::<String>::new()));
    let o = order.clone();
    bot.use_middleware(move |_c, _u, next| {
        o.lock().unwrap().push("before".into());
        let r = next();
        o.lock().unwrap().push("after".into());
        r
    });
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    bot.on_message(move |_c, _m| { h.fetch_add(1, Ordering::SeqCst); Ok(()) }, None);
    bot.start().unwrap();

    for i in 0..3 {
        mock.push(message_update(1, i, 42, "spam"));
    }
    mock.push(message_update(1, 10, 43, "other user"));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(hits.load(Ordering::SeqCst), 3, "2 from the spammer + 1 from another user");
    let o = order.lock().unwrap();
    // Middleware also wraps internal updates (3 authorization updates here), and
    // every "before" is paired with an "after" in strict alternation.
    assert_eq!(o.len(), 12);
    assert!(o.chunks(2).all(|p| p[0] == "before" && p[1] == "after"));
    bot.close();
}

#[test]
fn auto_retry_chat_not_found_and_flood_wait() {
    let mut deletes = 0;
    let mut leaves = 0;
    let script = server(move |req| match req.type_name() {
        "deleteMessages" => {
            deletes += 1;
            Some(vec![if deletes == 1 { td_error(req, 400, "Chat not found") } else { ok(req) }])
        }
        "getChat" => Some(vec![reply(req, Value::typed("chat").set("id", req.field("chat_id").int()).set("title", "T"))]),
        "leaveChat" => {
            leaves += 1;
            Some(vec![if leaves == 1 { td_error(req, 429, "Too Many Requests: retry after 1") } else { ok(req) }])
        }
        _ => None,
    });
    let (bot, mock) = boot(script, |o| {
        o.auto_retry = Some(AutoRetry { chat_not_found: true, message_not_found: true, max_flood_wait: Duration::from_secs(3) });
    });
    bot.start().unwrap();

    bot.delete_messages(5, &[1], true).unwrap();
    assert_eq!(mock.sent_of("deleteMessages").len(), 2);
    assert_eq!(mock.sent_of("getChat").len(), 1);

    let t = Instant::now();
    bot.leave_chat(5).unwrap();
    assert!(t.elapsed() >= Duration::from_millis(900), "should have slept out the flood wait");
    assert_eq!(mock.sent_of("leaveChat").len(), 2);
    bot.close();
}

#[test]
fn td_errors_flood_wait_surface_without_retry_and_timeouts() {
    let script = server(|req| match req.type_name() {
        "leaveChat" => Some(vec![td_error(req, 429, "Too Many Requests: retry after 9")]),
        _ => None,
    });
    let (bot, _mock) = boot(script, |o| {
        o.auto_retry = None;
        o.request_timeout = Duration::from_millis(200);
    });
    bot.start().unwrap();
    let e = bot.leave_chat(1).unwrap_err();
    assert!(e.is_flood_wait());
    assert_eq!(e.flood_wait_seconds(), 9);
    // the fake never answers unknown requests
    assert_eq!(bot.invoke(Value::typed("ghost")).unwrap_err(), Error::SendTimeout);
    bot.close();
}

#[test]
fn raw_invoke_json_and_formatting_requests() {
    let script = server(|req| match req.type_name() {
        "parseTextEntities" => Some(vec![reply(
            req,
            Value::typed("formattedText").set("text", "hi").set(
                "entities",
                vec![Value::typed("textEntity").set("offset", 0).set("length", 2).set("type", Value::typed("textEntityTypeBold"))],
            ),
        )]),
        _ => None,
    });
    let (bot, mock) = boot(script, |_| {});
    bot.start().unwrap();

    let me = bot.invoke_json(r#"{"@type":"getMe"}"#).unwrap();
    assert_eq!(me.field("id").int(), 99);
    assert!(bot.invoke_json("[]").is_err());

    let m = bot.send_text(3, "*hi*", &SendOpts::new().parse_mode("MarkdownV2").no_preview().silent()).unwrap();
    assert_eq!(m.chat_id(), 3);
    let parse = &mock.sent_of("parseTextEntities")[0];
    assert_eq!(parse.path(&["parse_mode", "@type"]).str(), "textParseModeMarkdown");
    assert_eq!(parse.path(&["parse_mode", "version"]).int(), 2);
    let send = &mock.sent_of("sendMessage")[0];
    assert!(send.path(&["input_message_content", "link_preview_options", "is_disabled"]).bool());
    assert!(send.path(&["options", "disable_notification"]).bool());
    assert_eq!(send.path(&["input_message_content", "text", "entities"]).items().len(), 1);

    // inline keyboard passes through as reply_markup
    let kb = inline_keyboard(vec![inline_row(vec![callback_button("A", "a:1")])]);
    bot.send_text(3, "x", &SendOpts::new().reply_markup(kb)).unwrap();
    assert_eq!(mock.sent_of("sendMessage")[1].path(&["reply_markup", "@type"]).str(), "replyMarkupInlineKeyboard");
    bot.close();
}

#[test]
fn connect_disconnect_and_close() {
    let (bot, mock) = boot(server(|_| None), |_| {});
    let (tx, rx) = channel::<&'static str>();
    let t = tx.clone();
    bot.on_connect(move |_c| { t.send("connect").unwrap(); Ok(()) });
    let t = tx.clone();
    bot.on_disconnect(move |_c| { t.send("disconnect").unwrap(); Ok(()) });
    bot.start().unwrap();

    mock.push(Value::typed("updateConnectionState").set("state", Value::typed("connectionStateReady")));
    assert_eq!(recv(&rx), "connect");
    bot.close();
    assert_eq!(recv(&rx), "disconnect");
    assert_eq!(bot.invoke(Value::typed("getMe")).unwrap_err(), Error::Closed);
    assert_eq!(mock.sent_of("close").len(), 1);
}

#[test]
fn update_handlers_for_arbitrary_types_and_removal() {
    let (bot, mock) = boot(server(|_| None), |_| {});
    let (tx, rx) = channel::<String>();
    let t = tx.clone();
    let h = bot.on_update("updateChatMember", move |_c, u| {
        t.send(u.0.field("chat_id").int().to_string()).unwrap();
        Ok(())
    });
    bot.start().unwrap();

    mock.push(Value::typed("updateChatMember").set("chat_id", 321));
    assert_eq!(recv(&rx), "321");
    assert!(bot.remove_handler(&h));
    assert!(!bot.remove_handler(&h));
    mock.push(Value::typed("updateChatMember").set("chat_id", 322));
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    bot.close();
}

#[test]
fn upload_bytes_stages_and_cleans_up() {
    let dir = std::env::temp_dir().join(format!("gotdbot-upload-{}", std::process::id()));
    let dir2 = dir.clone();
    let script = server(move |req| match req.type_name() {
        "uploadFile" => {
            // the staged file must exist while TDLib "reads" it
            let path = req.path(&["file", "path"]).str().to_string();
            assert!(std::path::Path::new(&path).exists(), "staged file missing: {path}");
            assert!(path.starts_with(&*std::fs::canonicalize(&dir2).unwrap().to_string_lossy()));
            Some(vec![reply(
                req,
                Value::typed("file").set("id", 55).set("remote", Value::typed("remoteFile").set("is_uploading_completed", true)),
            )])
        }
        _ => None,
    });
    let dir3 = dir.clone();
    let (bot, _mock) = boot(script, move |o| o.upload_dir = dir3.to_string_lossy().into_owned());
    bot.start().unwrap();

    let f = bot.input_file_bytes(b"hello", "../../evil/name.txt").unwrap();
    assert_eq!(f.to_json(), r#"{"@type":"inputFileId","id":55}"#);
    assert!(!dir.join("name.txt").exists(), "staged file should be removed");
    assert!(bot.upload_bytes(b"", "x").is_err());
    let _ = std::fs::remove_dir_all(&dir);
    bot.close();
}
