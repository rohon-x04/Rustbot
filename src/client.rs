//! The TDLib client: authorization, request/response correlation, update
//! dispatch, handler registration, middleware and conversation waiters.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, Once, RwLock};
use std::thread;
use std::time::Duration;

use crate::crypto::base64_encode;
use crate::error::{Error, HandlerResult, Result, TdError};
use crate::filters::callback::CallbackFilter;
use crate::filters::message::MessageFilter;
use crate::handlers::{CallbackHandler, CommandHandler, Handler, MessageHandler, Plugin, UpdateHandler};
use crate::json::Value;
use crate::logger::{Level, Logger};
use crate::tdjson::{Backend, TdJson};
use crate::types::{CallbackQuery, Message, Update, User};

/// Library version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Seconds to wait for TDLib to answer a request before `Error::SendTimeout`.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Automatic recovery from common TDLib errors.
#[derive(Clone, Debug, Default)]
pub struct AutoRetry {
    /// On "Chat not found", call `getChat` and retry once.
    pub chat_not_found: bool,
    /// On "Message not found", call `getMessage` and retry once.
    pub message_not_found: bool,
    /// Sleep out flood waits up to this long, then retry. Zero disables.
    pub max_flood_wait: Duration,
}

/// Client configuration. Use `ClientOpts::default()` and override fields.
#[derive(Clone)]
pub struct ClientOpts {
    /// Path to `libtdjson` (empty = OS default name).
    pub library_path: String,
    pub use_test_dc: bool,
    pub database_directory: String,
    pub files_directory: String,
    /// Where `upload_bytes` stages files. Ignored when `files_directory` is set.
    pub upload_dir: String,
    pub database_encryption_key: String,
    pub use_file_database: bool,
    pub use_chat_info_database: bool,
    pub use_message_database: bool,
    pub use_secret_chats: bool,
    pub system_language_code: String,
    pub device_model: String,
    pub system_version: String,
    pub application_version: String,
    /// TDLib options set (via `setOption`) before authorization, e.g.
    /// `("ignore_background_updates".into(), Value::Bool(true))`.
    pub tdlib_options: Vec<(String, Value)>,
    pub logger: Arc<Logger>,
    /// Log in with a QR code (user accounts).
    pub qr_mode: bool,
    pub authorization_timeout: Duration,
    /// TDLib's own log verbosity (`-1` leaves it untouched).
    pub log_verbosity_level: i32,
    pub auto_retry: Option<AutoRetry>,
    /// Default parse mode for text: `""`, `"html"`, `"markdown"` or `"markdownv2"`.
    pub parse_mode: String,
    /// Secret for signed callback data.
    pub callback_secret: Vec<u8>,
    pub request_timeout: Duration,
    /// Use a custom backend instead of loading `libtdjson` (tests, mocks).
    pub backend: Option<Arc<dyn Backend>>,
}

impl Default for ClientOpts {
    fn default() -> Self {
        ClientOpts {
            library_path: String::new(),
            use_test_dc: false,
            database_directory: "database".into(),
            files_directory: String::new(),
            upload_dir: String::new(),
            database_encryption_key: String::new(),
            use_file_database: true,
            use_chat_info_database: true,
            use_message_database: true,
            use_secret_chats: false,
            system_language_code: "en".into(),
            device_model: "Gotdbot".into(),
            system_version: std::env::consts::OS.into(),
            application_version: format!("Gotdbot {VERSION}"),
            tdlib_options: Vec::new(),
            logger: Arc::new(Logger::new(Level::Info)),
            qr_mode: false,
            authorization_timeout: Duration::from_secs(60),
            log_verbosity_level: 2,
            auto_retry: Some(AutoRetry::default()),
            parse_mode: String::new(),
            callback_secret: Vec::new(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            backend: None,
        }
    }
}

/// Middleware wraps handler dispatch. Call `next()` to continue.
pub type Middleware = Arc<dyn for<'a> Fn(&Client, &Update, &'a dyn Fn() -> HandlerResult) -> HandlerResult + Send + Sync>;
/// Called when a handler returns a non-control error. The returned value decides
/// what dispatch does next (see the handler docs).
pub type ErrorHandler = Arc<dyn Fn(&Client, &Update, Error) -> HandlerResult + Send + Sync>;
/// Called when a handler panics, with the panic message.
pub type PanicHandler = Arc<dyn Fn(&Client, &Update, String) + Send + Sync>;
type LifecycleFn = Arc<dyn Fn(&Client) -> HandlerResult + Send + Sync>;

#[derive(Default)]
struct HandlersData {
    groups: BTreeMap<i32, Vec<Arc<dyn Handler>>>,
}

struct Waiter {
    filter: Arc<dyn Fn(&Client, &Update) -> bool + Send + Sync>,
    tx: Sender<Update>,
}

#[derive(Default)]
struct SendTracker {
    waiters: HashMap<String, Sender<Value>>,
    done: HashMap<String, Value>,
    order: VecDeque<String>,
}

#[derive(Default)]
struct Lifecycle {
    ready: bool,
    connect: Vec<LifecycleFn>,
    disconnect: Vec<LifecycleFn>,
}

// ---- ClientManager -----------------------------------------------------------

#[derive(Default)]
struct ManagerState {
    clients: HashMap<i32, Arc<Inner>>,
    running: bool,
}

/// Owns the single `td_receive` loop of a TDLib backend and routes incoming
/// JSON to the right [`Client`].
pub struct ClientManager {
    backend: Arc<dyn Backend>,
    state: Mutex<ManagerState>,
}

static DEFAULT_MANAGER: Mutex<Option<Arc<ClientManager>>> = Mutex::new(None);

impl ClientManager {
    /// A manager over any backend.
    pub fn new(backend: Arc<dyn Backend>) -> Arc<ClientManager> {
        Arc::new(ClientManager { backend, state: Mutex::new(ManagerState::default()) })
    }

    /// The process-wide manager for the real `libtdjson` (loaded on first use).
    pub fn default_for(library_path: &str) -> Result<Arc<ClientManager>> {
        let mut g = DEFAULT_MANAGER.lock().unwrap();
        if let Some(m) = g.as_ref() {
            return Ok(m.clone());
        }
        let m = ClientManager::new(Arc::new(TdJson::load(library_path)?));
        *g = Some(m.clone());
        Ok(m)
    }

    pub fn backend(&self) -> &Arc<dyn Backend> {
        &self.backend
    }

    fn add(self: &Arc<Self>, inner: Arc<Inner>) {
        let mut st = self.state.lock().unwrap();
        st.clients.insert(inner.client_id, inner);
        if !st.running {
            st.running = true;
            let me = self.clone();
            thread::Builder::new()
                .name("gotdbot-receiver".into())
                .spawn(move || me.receive_loop())
                .expect("spawn receiver thread");
        }
    }

    fn remove(&self, client_id: i32) {
        self.state.lock().unwrap().clients.remove(&client_id);
    }

    fn receive_loop(self: Arc<Self>) {
        loop {
            {
                let mut st = self.state.lock().unwrap();
                if st.clients.is_empty() {
                    st.running = false;
                    return;
                }
            }
            let raw = match self.backend.receive(0.1) {
                Some(r) => r,
                None => continue,
            };
            let mut v = match Value::parse(&raw) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let client_id = v.remove("@client_id").and_then(|c| c.as_i64()).unwrap_or(0) as i32;
            let target = self.state.lock().unwrap().clients.get(&client_id).cloned();
            if let Some(inner) = target {
                inner.route(v);
            }
        }
    }
}

// ---- Client ----------------------------------------------------------------------

pub(crate) struct Inner {
    manager: Arc<ClientManager>,
    pub(crate) client_id: i32,
    api_id: i32,
    api_hash: String,
    bot_token: String,
    phone: String,
    pub(crate) config: ClientOpts,
    pub(crate) logger: Arc<Logger>,

    pending: Mutex<HashMap<String, Sender<Value>>>,
    sends: Mutex<SendTracker>,
    updates_tx: Mutex<Sender<Update>>,
    updates_rx: Mutex<Option<Receiver<Update>>>,

    handlers: RwLock<Arc<HandlersData>>,
    middlewares: Mutex<Vec<Middleware>>,
    error_handler: Mutex<Option<ErrorHandler>>,
    panic_handler: Mutex<Option<PanicHandler>>,
    waiters: Mutex<HashMap<i64, HashMap<u64, Waiter>>>,
    waiter_seq: AtomicU64,
    lifecycle: Mutex<Lifecycle>,

    auth_tx: Mutex<Sender<Result<()>>>,
    auth_rx: Mutex<Receiver<Result<()>>>,
    authorized: AtomicBool,
    started: Once,
    close_once: Once,
    closed: (Mutex<bool>, Condvar),
    stop: AtomicBool,
    request_id: AtomicU64,
    me: RwLock<Option<User>>,
}

/// A TDLib client. Cheap to clone (all clones share the same connection).
#[derive(Clone)]
pub struct Client {
    pub(crate) inner: Arc<Inner>,
}

impl Inner {
    /// Called from the receiver thread for every JSON object of this client.
    fn route(self: &Arc<Self>, mut v: Value) {
        let extra = v.remove("@extra");
        if let Some(extra) = extra.as_ref().and_then(|e| e.as_str().map(str::to_string)) {
            if let Some(tx) = self.pending.lock().unwrap().remove(&extra) {
                let _ = tx.send(v);
                return;
            }
        }
        // Resolve `wait_message` waiters inline: it only touches a map, and doing it
        // here (not on a handler thread) closes the window in which a fast
        // "send succeeded" update could be missed.
        match v.type_name() {
            "updateMessageSendSucceeded" | "updateMessageSendFailed" => {
                let m = v.field("message");
                let key = format!("{}:{}", m.field("chat_id").int(), v.field("old_message_id").int());
                self.deliver_send_result(key, v.clone());
            }
            _ => {}
        }
        let _ = self.updates_tx.lock().unwrap().send(Update(v));
    }

    fn deliver_send_result(&self, key: String, v: Value) {
        let mut st = self.sends.lock().unwrap();
        if let Some(tx) = st.waiters.remove(&key) {
            let _ = tx.send(v);
        } else {
            st.done.insert(key.clone(), v);
            st.order.push_back(key);
            while st.order.len() > 512 {
                if let Some(old) = st.order.pop_front() {
                    st.done.remove(&old);
                }
            }
        }
    }
}

fn prompt(msg: &str) -> String {
    print!("{msg}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    line.trim().to_string()
}

fn looks_like_bot_token(s: &str) -> bool {
    match s.split_once(':') {
        Some((id, rest)) => {
            !id.is_empty()
                && id.bytes().all(|b| b.is_ascii_digit())
                && !rest.is_empty()
                && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        }
        None => false,
    }
}

impl Client {
    /// Create a client. `token_or_phone` is a bot token (`123:ABC...`) or a phone
    /// number for user accounts. Nothing is sent until [`start`](Self::start).
    pub fn new(api_id: i32, api_hash: &str, token_or_phone: &str, opts: ClientOpts) -> Result<Client> {
        let manager = match &opts.backend {
            Some(b) => ClientManager::new(b.clone()),
            None => ClientManager::default_for(&opts.library_path)?,
        };
        if opts.log_verbosity_level >= 0 {
            manager.backend.execute(&format!(
                r#"{{"@type":"setLogVerbosityLevel","new_verbosity_level":{}}}"#,
                opts.log_verbosity_level
            ));
        }
        let token_or_phone = token_or_phone.trim();
        let (bot_token, phone) = if looks_like_bot_token(token_or_phone) {
            (token_or_phone.to_string(), String::new())
        } else {
            (String::new(), token_or_phone.to_string())
        };
        let (utx, urx) = channel();
        let (atx, arx) = channel();
        let logger = opts.logger.clone();
        let inner = Arc::new(Inner {
            client_id: manager.backend.create_client_id(),
            manager,
            api_id,
            api_hash: api_hash.to_string(),
            bot_token,
            phone,
            config: opts,
            logger,
            pending: Mutex::new(HashMap::new()),
            sends: Mutex::new(SendTracker::default()),
            updates_tx: Mutex::new(utx),
            updates_rx: Mutex::new(Some(urx)),
            handlers: RwLock::new(Arc::new(HandlersData::default())),
            middlewares: Mutex::new(Vec::new()),
            error_handler: Mutex::new(None),
            panic_handler: Mutex::new(None),
            waiters: Mutex::new(HashMap::new()),
            waiter_seq: AtomicU64::new(0),
            lifecycle: Mutex::new(Lifecycle::default()),
            auth_tx: Mutex::new(atx),
            auth_rx: Mutex::new(arx),
            authorized: AtomicBool::new(false),
            started: Once::new(),
            close_once: Once::new(),
            closed: (Mutex::new(false), Condvar::new()),
            stop: AtomicBool::new(false),
            request_id: AtomicU64::new(0),
            me: RwLock::new(None),
        });
        Ok(Client { inner })
    }

    // ---- lifecycle ---------------------------------------------------------------

    /// The client's logger.
    pub fn logger(&self) -> &Arc<Logger> {
        &self.inner.logger
    }

    /// The client's configuration.
    pub fn config(&self) -> &ClientOpts {
        &self.inner.config
    }

    /// The logged-in account (set after authorization).
    pub fn me(&self) -> Option<User> {
        self.inner.me.read().unwrap().clone()
    }

    /// Start TDLib and block until authorization succeeds or fails.
    pub fn start(&self) -> Result<()> {
        self.init_client();
        let rx = self.inner.auth_rx.lock().unwrap();
        match rx.recv_timeout(self.inner.config.authorization_timeout) {
            Ok(r) => r,
            Err(_) => Err(Error::Auth("authorization timeout".into())),
        }
    }

    /// Start the TDLib instance without waiting for authorization. Useful to call
    /// unauthenticated TDLib methods before [`start`](Self::start).
    pub fn send_dummy_request(&self) {
        self.init_client();
    }

    fn init_client(&self) {
        let me = self.clone();
        self.inner.started.call_once(move || {
            let inner = me.inner.clone();
            inner.manager.add(inner.clone());
            inner.manager.backend.send(inner.client_id, r#"{"@type":"getOption","name":"version"}"#);
            let rx = inner.updates_rx.lock().unwrap().take().expect("updates receiver");
            let client = me.clone();
            thread::Builder::new()
                .name("gotdbot-processor".into())
                .spawn(move || client.processor(rx))
                .expect("spawn processor thread");
        });
    }

    /// Block until SIGINT/SIGTERM (or until the client closes), then close.
    pub fn idle(&self) {
        crate::signal::wait_for_termination(|| self.is_closed());
        self.close();
    }

    fn is_closed(&self) -> bool {
        *self.inner.closed.0.lock().unwrap()
    }

    /// Close TDLib: databases are flushed, then the client stops.
    pub fn close(&self) {
        let me = self.clone();
        self.inner.close_once.call_once(move || {
            if !me.inner.started.is_completed() {
                return;
            }
            let _ = me.emit_disconnect();
            let _ = me.invoke_timeout(Value::typed("close"), Duration::from_secs(5), false);
            let (lock, cv) = &me.inner.closed;
            let guard = lock.lock().unwrap();
            let (g, res) = cv.wait_timeout_while(guard, Duration::from_secs(5), |c| !*c).unwrap();
            drop(g);
            if res.timed_out() {
                me.inner.logger.warn("Timeout waiting for TDLib to close", &[]);
            }
            me.inner.stop.store(true, Ordering::SeqCst);
            me.inner.manager.remove(me.inner.client_id);
        });
    }

    // ---- requests --------------------------------------------------------------------

    /// Send a raw TDLib request (a JSON object with `@type`) and wait for the
    /// response. TDLib errors become `Err(Error::Td(..))`.
    ///
    /// This is the escape hatch for any TDLib method, including ones newer than
    /// this crate: `client.invoke(Value::typed("getMe"))`.
    pub fn invoke(&self, req: Value) -> Result<Value> {
        self.invoke_timeout(req, self.inner.config.request_timeout, true)
    }

    /// [`invoke`](Self::invoke) with hand-written JSON.
    pub fn invoke_json(&self, request: &str) -> Result<Value> {
        let v = Value::parse(request.trim())?;
        if v.as_object().is_none() {
            return Err(Error::Json("request must be a JSON object".into()));
        }
        self.invoke(v)
    }

    /// [`invoke`](Self::invoke) with an explicit timeout and optional auto-retry.
    pub fn invoke_timeout(&self, mut req: Value, timeout: Duration, auto_retry: bool) -> Result<Value> {
        if req.as_object().is_none() {
            return Err(Error::Json("request must be a JSON object".into()));
        }
        let req_type = req.type_name().to_ascii_lowercase();
        let mut chat_attempted = req_type == "getchat";
        let mut message_attempted = matches!(
            req_type.as_str(),
            "getmessage" | "getmessagelocally" | "getrepliedmessage" | "getcallbackquerymessage"
        );
        loop {
            if self.is_closed() && req_type != "close" {
                return Err(Error::Closed);
            }
            let extra = self.inner.request_id.fetch_add(1, Ordering::SeqCst) + 1;
            let extra = extra.to_string();
            req.insert("@extra", extra.as_str());
            let (tx, rx) = channel();
            self.inner.pending.lock().unwrap().insert(extra.clone(), tx);
            self.inner.manager.backend.send(self.inner.client_id, &req.to_json());

            let resp = match rx.recv_timeout(timeout) {
                Ok(v) => v,
                Err(_) => {
                    self.inner.pending.lock().unwrap().remove(&extra);
                    return Err(Error::SendTimeout);
                }
            };
            if resp.type_name() != "error" {
                return Ok(resp);
            }
            let err = TdError { code: resp.field("code").int() as i32, message: resp.field("message").str().to_string() };
            if auto_retry && self.handle_auto_retry(&req, &err, &mut chat_attempted, &mut message_attempted) {
                continue;
            }
            return Err(Error::Td(err));
        }
    }

    fn handle_auto_retry(&self, req: &Value, err: &TdError, chat_tried: &mut bool, msg_tried: &mut bool) -> bool {
        let cfg = match &self.inner.config.auto_retry {
            Some(c) => c,
            None => return false,
        };
        if err.code == 429 && !cfg.max_flood_wait.is_zero() {
            let secs = err.retry_after();
            if secs > 0 && Duration::from_secs(secs) <= cfg.max_flood_wait {
                self.inner.logger.info("Flood wait", &[("seconds", &secs)]);
                thread::sleep(Duration::from_secs(secs));
                return true;
            }
        }
        if err.code != 400 {
            return false;
        }
        let chat_id = req.field("chat_id").int();
        let message_id = req.field("message_id").int();
        if !*msg_tried && err.message == "Message not found" && cfg.message_not_found {
            *msg_tried = true;
            if chat_id != 0 && message_id != 0 && self.get_message(chat_id, message_id).is_ok() {
                return true;
            }
        }
        if !*chat_tried && err.message == "Chat not found" && cfg.chat_not_found {
            *chat_tried = true;
            if chat_id != 0 && self.get_chat(chat_id).is_ok() {
                let reply_to = req.path(&["reply_to", "message_id"]).int();
                if reply_to > 0 {
                    let _ = self.get_message(chat_id, reply_to);
                }
                return true;
            }
        }
        false
    }

    /// Wait until a just-sent message leaves the "pending" state and return the
    /// final message (or the send error).
    pub(crate) fn wait_message(&self, msg: Message) -> Result<Message> {
        if msg.0.path(&["sending_state", "@type"]).str() != "messageSendingStatePending" {
            return Ok(msg);
        }
        let key = format!("{}:{}", msg.chat_id(), msg.id());
        let rx = {
            let mut st = self.inner.sends.lock().unwrap();
            if let Some(done) = st.done.remove(&key) {
                drop(st);
                return Self::send_outcome(done, msg);
            }
            let (tx, rx) = channel();
            st.waiters.insert(key.clone(), tx);
            rx
        };
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(v) => Self::send_outcome(v, msg),
            Err(_) => {
                self.inner.sends.lock().unwrap().waiters.remove(&key);
                Ok(msg)
            }
        }
    }

    fn send_outcome(update: Value, original: Message) -> Result<Message> {
        match update.type_name() {
            "updateMessageSendSucceeded" => Ok(Message(update.field("message").clone())),
            "updateMessageSendFailed" => {
                let e = update.field("error");
                Err(Error::Td(TdError { code: e.field("code").int() as i32, message: e.field("message").str().to_string() }))
            }
            _ => Ok(original),
        }
    }

    // ---- handler registration -------------------------------------------------------------

    /// Register a handler in group 0.
    pub fn add_handler(&self, handler: impl Handler + 'static) -> Arc<dyn Handler> {
        self.add_handler_group(handler, 0)
    }

    /// Register a handler in `group` (lower groups run first).
    pub fn add_handler_group(&self, handler: impl Handler + 'static, group: i32) -> Arc<dyn Handler> {
        let h: Arc<dyn Handler> = Arc::new(handler);
        self.add_arc_handler(h.clone(), group);
        h
    }

    pub(crate) fn add_arc_handler(&self, handler: Arc<dyn Handler>, group: i32) {
        let mut guard = self.inner.handlers.write().unwrap();
        let mut next = HandlersData { groups: guard.groups.clone() };
        next.groups.entry(group).or_default().push(handler);
        *guard = Arc::new(next);
    }

    /// Remove a handler (by identity) from group 0.
    pub fn remove_handler(&self, handler: &Arc<dyn Handler>) -> bool {
        self.remove_handler_group(handler, 0)
    }

    /// Remove a handler (by identity) from `group`.
    pub fn remove_handler_group(&self, handler: &Arc<dyn Handler>, group: i32) -> bool {
        let mut guard = self.inner.handlers.write().unwrap();
        let mut next = HandlersData { groups: guard.groups.clone() };
        let list = match next.groups.get_mut(&group) {
            Some(l) => l,
            None => return false,
        };
        let idx = match list.iter().position(|h| Arc::ptr_eq(h, handler)) {
            Some(i) => i,
            None => return false,
        };
        list.remove(idx);
        if list.is_empty() {
            next.groups.remove(&group);
        }
        *guard = Arc::new(next);
        true
    }

    /// Handle `/name` commands.
    pub fn on_command(
        &self,
        command: &str,
        handler: impl Fn(&Client, &Message) -> HandlerResult + Send + Sync + 'static,
    ) -> Arc<dyn Handler> {
        self.add_handler(CommandHandler::new(command, handler))
    }

    /// Handle `/name` commands in a specific group.
    pub fn on_command_group(
        &self,
        command: &str,
        handler: impl Fn(&Client, &Message) -> HandlerResult + Send + Sync + 'static,
        group: i32,
    ) -> Arc<dyn Handler> {
        self.add_handler_group(CommandHandler::new(command, handler), group)
    }

    /// Handle new messages, optionally filtered.
    pub fn on_message(
        &self,
        handler: impl Fn(&Client, &Message) -> HandlerResult + Send + Sync + 'static,
        filter: Option<MessageFilter>,
    ) -> Arc<dyn Handler> {
        self.add_handler(MessageHandler::new(handler, filter))
    }

    /// Handle new messages in a specific group.
    pub fn on_message_group(
        &self,
        handler: impl Fn(&Client, &Message) -> HandlerResult + Send + Sync + 'static,
        filter: Option<MessageFilter>,
        group: i32,
    ) -> Arc<dyn Handler> {
        self.add_handler_group(MessageHandler::new(handler, filter), group)
    }

    /// Handle every pressed inline button.
    pub fn on_callback_query(
        &self,
        handler: impl Fn(&Client, &CallbackQuery) -> HandlerResult + Send + Sync + 'static,
        filter: Option<CallbackFilter>,
    ) -> Arc<dyn Handler> {
        self.add_handler(CallbackHandler::new(None, handler, filter))
    }

    /// Handle inline buttons whose data is `action` or `action:args...`.
    pub fn on_callback(
        &self,
        action: &str,
        handler: impl Fn(&Client, &CallbackQuery) -> HandlerResult + Send + Sync + 'static,
        filter: Option<CallbackFilter>,
    ) -> Arc<dyn Handler> {
        self.add_handler(CallbackHandler::new(Some(action), handler, filter))
    }

    /// Handle updates of one TDLib type (e.g. `"updateChatMember"`); an empty
    /// `type_name` receives every update. If a catch-all must not swallow typed
    /// handlers in the same group, return `Err(Error::ContinueHandlers)`.
    pub fn on_update(
        &self,
        type_name: &str,
        handler: impl Fn(&Client, &Update) -> HandlerResult + Send + Sync + 'static,
    ) -> Arc<dyn Handler> {
        self.add_handler(UpdateHandler::new(type_name, handler, None))
    }

    /// Handle every raw update (see [`on_update`](Self::on_update)).
    pub fn on_raw_update(
        &self,
        handler: impl Fn(&Client, &Update) -> HandlerResult + Send + Sync + 'static,
    ) -> Arc<dyn Handler> {
        self.on_update("", handler)
    }

    /// Load all handlers of a plugin.
    pub fn add_plugin(&self, plugin: Plugin) {
        for (h, g) in &plugin.handlers {
            self.add_arc_handler(h.clone(), *g);
        }
        self.inner.logger.info("Loaded plugin", &[("name", &plugin.name), ("handlers", &plugin.handlers.len())]);
    }

    /// Add middleware (outermost first).
    pub fn use_middleware(
        &self,
        mw: impl for<'a> Fn(&Client, &Update, &'a dyn Fn() -> HandlerResult) -> HandlerResult + Send + Sync + 'static,
    ) {
        self.inner.middlewares.lock().unwrap().push(Arc::new(mw));
    }

    /// Install the handler for non-control errors returned by handlers.
    pub fn on_error(&self, handler: impl Fn(&Client, &Update, Error) -> HandlerResult + Send + Sync + 'static) {
        *self.inner.error_handler.lock().unwrap() = Some(Arc::new(handler));
    }

    /// Install the handler called when a handler panics.
    pub fn on_panic(&self, handler: impl Fn(&Client, &Update, String) + Send + Sync + 'static) {
        *self.inner.panic_handler.lock().unwrap() = Some(Arc::new(handler));
    }

    /// Run `f` once the connection to Telegram is ready.
    pub fn on_connect(&self, f: impl Fn(&Client) -> HandlerResult + Send + Sync + 'static) {
        self.inner.lifecycle.lock().unwrap().connect.push(Arc::new(f));
    }

    /// Run `f` when the connection drops or the client closes.
    pub fn on_disconnect(&self, f: impl Fn(&Client) -> HandlerResult + Send + Sync + 'static) {
        self.inner.lifecycle.lock().unwrap().disconnect.push(Arc::new(f));
    }

    fn emit_connect(&self) -> HandlerResult {
        let hs = {
            let mut l = self.inner.lifecycle.lock().unwrap();
            if l.ready {
                return Ok(());
            }
            l.ready = true;
            l.connect.clone()
        };
        for h in hs {
            h(self)?;
        }
        Ok(())
    }

    fn emit_disconnect(&self) -> HandlerResult {
        let hs = {
            let mut l = self.inner.lifecycle.lock().unwrap();
            if !l.ready {
                return Ok(());
            }
            l.ready = false;
            l.disconnect.clone()
        };
        for h in hs {
            h(self)?;
        }
        Ok(())
    }

    // ---- waiters (conversations) -------------------------------------------------------------

    /// Block until an update matching `filter` arrives (any chat) or `timeout` passes.
    pub fn wait_for(
        &self,
        filter: impl Fn(&Client, &Update) -> bool + Send + Sync + 'static,
        timeout: Duration,
    ) -> Result<Update> {
        self.wait_for_chat(0, filter, timeout)
    }

    /// Like [`wait_for`](Self::wait_for) but only for updates of `chat_id`
    /// (`0` = every chat).
    pub fn wait_for_chat(
        &self,
        chat_id: i64,
        filter: impl Fn(&Client, &Update) -> bool + Send + Sync + 'static,
        timeout: Duration,
    ) -> Result<Update> {
        let (tx, rx) = channel();
        let id = self.inner.waiter_seq.fetch_add(1, Ordering::SeqCst) + 1;
        self.inner
            .waiters
            .lock()
            .unwrap()
            .entry(chat_id)
            .or_default()
            .insert(id, Waiter { filter: Arc::new(filter), tx });

        struct Guard<'a> {
            client: &'a Client,
            chat_id: i64,
            id: u64,
        }
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                let mut w = self.client.inner.waiters.lock().unwrap();
                if let Some(inner) = w.get_mut(&self.chat_id) {
                    inner.remove(&self.id);
                    if inner.is_empty() {
                        w.remove(&self.chat_id);
                    }
                }
            }
        }
        let _guard = Guard { client: self, chat_id, id };
        rx.recv_timeout(timeout).map_err(|_| Error::ConversationTimeout)
    }

    // ---- dispatch -------------------------------------------------------------------------------

    fn processor(&self, rx: Receiver<Update>) {
        while !self.inner.stop.load(Ordering::SeqCst) {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(update) => {
                    let client = self.clone();
                    let _ = thread::Builder::new().name("gotdbot-update".into()).spawn(move || client.dispatch(update));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return,
            }
        }
    }

    fn dispatch(&self, update: Update) {
        let result = catch_unwind(AssertUnwindSafe(|| self.dispatch_inner(&update)));
        if let Err(p) = result {
            let msg = if let Some(s) = p.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = p.downcast_ref::<String>() {
                s.clone()
            } else {
                "unknown panic".to_string()
            };
            let handler = self.inner.panic_handler.lock().unwrap().clone();
            match handler {
                Some(h) => h(self, &update, msg),
                None => self.inner.logger.error("Panic in handler", &[("panic", &msg)]),
            }
        }
    }

    fn dispatch_inner(&self, update: &Update) {
        self.internal_update(update);
        self.notify_waiters(update);

        let chain: Vec<Middleware> = self.inner.middlewares.lock().unwrap().clone();
        let end = || self.run_handlers(update);
        let result = run_chain(self, update, &chain, &end);
        if let Err(e) = result {
            let _ = self.handle_error(update, Err(e));
        }
    }

    fn notify_waiters(&self, update: &Update) {
        let matched: Vec<(Arc<dyn Fn(&Client, &Update) -> bool + Send + Sync>, Sender<Update>)> = {
            let w = self.inner.waiters.lock().unwrap();
            if w.is_empty() {
                return;
            }
            let chat_id = update.chat_id();
            let mut keys = vec![0i64];
            if chat_id != 0 {
                keys.push(chat_id);
            }
            let mut out = Vec::new();
            for key in keys {
                if let Some(inner) = w.get(&key) {
                    out.extend(inner.values().map(|x| (x.filter.clone(), x.tx.clone())));
                }
            }
            out
        };
        for (filter, tx) in matched {
            if filter(self, update) {
                let _ = tx.send(update.clone());
            }
        }
    }

    fn handle_error(&self, update: &Update, result: HandlerResult) -> HandlerResult {
        let err = match result {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };
        if err.is_control() {
            return Err(err);
        }
        let handler = self.inner.error_handler.lock().unwrap().clone();
        match handler {
            Some(h) => h(self, update, err),
            None => {
                self.inner.logger.error("Handler error", &[("error", &err), ("type", &update.kind())]);
                Ok(())
            }
        }
    }

    fn run_handlers(&self, update: &Update) -> HandlerResult {
        let data = self.inner.handlers.read().unwrap().clone();
        for handlers in data.groups.values() {
            for h in handlers {
                if !h.check_update(self, update) {
                    continue;
                }
                let action = self.handle_error(update, h.handle_update(self, update));
                match action {
                    Err(Error::StopPropagation) => return Err(Error::StopPropagation),
                    Err(Error::ContinueGroups) => break,
                    Err(Error::ContinueHandlers) => continue,
                    _ => return Ok(()),
                }
            }
        }
        Ok(())
    }

    // ---- built-in update handling ------------------------------------------------------------------

    fn internal_update(&self, update: &Update) {
        match update.kind() {
            "updateAuthorizationState" => self.on_authorization_state(update.0.field("authorization_state")),
            "updateConnectionState" => {
                let state = update.0.path(&["state", "@type"]).str().trim_start_matches("connectionState").to_string();
                self.inner.logger.info("Connection state changed", &[("state", &state)]);
                let _ = match state.as_str() {
                    "Ready" => self.emit_connect(),
                    "WaitingForNetwork" => self.emit_disconnect(),
                    _ => Ok(()),
                };
            }
            "updateMessageSendFailed" => {
                let m = update.0.field("message");
                let (chat_id, id) = (m.field("chat_id").int(), m.field("id").int());
                if id != 0 {
                    let c = self.clone();
                    thread::spawn(move || {
                        let _ = c.delete_messages(chat_id, &[id], false);
                    });
                }
            }
            _ => {}
        }
    }

    fn send_auth_result(&self, r: Result<()>) {
        let _ = self.inner.auth_tx.lock().unwrap().send(r);
    }

    fn auth_call(&self, req: Value) {
        if let Err(e) = self.invoke(req) {
            self.inner.logger.error("Authorization step failed", &[("error", &e)]);
            self.send_auth_result(Err(e));
        }
    }

    fn on_authorization_state(&self, state: &Value) {
        let cfg = &self.inner.config;
        let log = &self.inner.logger;
        log.debug("Authorization state update", &[("state", &state.type_name())]);
        match state.type_name() {
            "authorizationStateWaitTdlibParameters" => {
                for (name, value) in &cfg.tdlib_options {
                    let ov = match value {
                        Value::Bool(b) => Value::typed("optionValueBoolean").set("value", *b),
                        Value::Int(i) => Value::typed("optionValueInteger").set("value", *i),
                        Value::Str(s) => Value::typed("optionValueString").set("value", s.as_str()),
                        _ => Value::typed("optionValueEmpty"),
                    };
                    if let Err(e) = self.invoke(Value::typed("setOption").set("name", name.as_str()).set("value", ov)) {
                        log.error("Error setting option", &[("option", name), ("error", &e)]);
                    }
                }
                self.auth_call(
                    Value::typed("setTdlibParameters")
                        .set("use_test_dc", cfg.use_test_dc)
                        .set("database_directory", cfg.database_directory.as_str())
                        .set("files_directory", cfg.files_directory.as_str())
                        .set("database_encryption_key", base64_encode(cfg.database_encryption_key.as_bytes()))
                        .set("use_file_database", cfg.use_file_database)
                        .set("use_chat_info_database", cfg.use_chat_info_database)
                        .set("use_message_database", cfg.use_message_database)
                        .set("use_secret_chats", cfg.use_secret_chats)
                        .set("api_id", self.inner.api_id)
                        .set("api_hash", self.inner.api_hash.as_str())
                        .set("system_language_code", cfg.system_language_code.as_str())
                        .set("device_model", cfg.device_model.as_str())
                        .set("system_version", cfg.system_version.as_str())
                        .set("application_version", cfg.application_version.as_str()),
                );
            }
            "authorizationStateWaitPhoneNumber" => {
                if !self.inner.bot_token.is_empty() {
                    self.auth_call(Value::typed("checkAuthenticationBotToken").set("token", self.inner.bot_token.as_str()));
                } else if cfg.qr_mode {
                    self.auth_call(Value::typed("requestQrCodeAuthentication").set("other_user_ids", Vec::<i64>::new()));
                } else {
                    let phone = if self.inner.phone.is_empty() { prompt("Enter phone number: ") } else { self.inner.phone.clone() };
                    self.auth_call(Value::typed("setAuthenticationPhoneNumber").set("phone_number", phone));
                }
            }
            "authorizationStateWaitOtherDeviceConfirmation" => {
                println!("Open this link in Telegram on a logged-in device to confirm the login:\n{}", state.field("link").str());
            }
            "authorizationStateWaitCode" => {
                let kind = state.path(&["code_info", "type", "@type"]).str().trim_start_matches("authenticationCodeType").to_string();
                loop {
                    let code = prompt(&format!("Enter the code received via {kind}: "));
                    if code.is_empty() {
                        continue;
                    }
                    match self.invoke(Value::typed("checkAuthenticationCode").set("code", code)) {
                        Ok(_) => break,
                        Err(e) => println!("Error checking code: {e}"),
                    }
                }
            }
            "authorizationStateWaitPassword" => {
                let hint = state.field("password_hint").str().to_string();
                loop {
                    let pw = prompt(&format!("Enter your 2FA password (hint: {hint}): "));
                    if pw.is_empty() {
                        continue;
                    }
                    match self.invoke(Value::typed("checkAuthenticationPassword").set("password", pw)) {
                        Ok(_) => break,
                        Err(e) => println!("Error checking password: {e}"),
                    }
                }
            }
            "authorizationStateWaitRegistration" => {
                let first = prompt("Enter first name: ");
                let last = prompt("Enter last name: ");
                self.auth_call(Value::typed("registerUser").set("first_name", first).set("last_name", last));
            }
            "authorizationStatePremiumPurchase" | "authorizationStateWaitPremiumPurchase" => {
                log.warn("Account is limited and requires Telegram Premium.", &[]);
                self.send_auth_result(Err(Error::WaitPremiumPurchase));
            }
            "authorizationStateReady" => {
                self.inner.authorized.store(true, Ordering::SeqCst);
                match self.get_me() {
                    Ok(me) => {
                        let (id, uname) = (me.id(), me.username().to_string());
                        *self.inner.me.write().unwrap() = Some(me);
                        log.info("Logged in", &[("user_id", &id), ("username", &uname)]);
                        self.send_auth_result(Ok(()));
                    }
                    Err(e) => {
                        log.error("Failed to get me", &[("error", &e)]);
                        self.send_auth_result(Err(e));
                    }
                }
            }
            "authorizationStateClosed" => {
                if !self.inner.authorized.load(Ordering::SeqCst) {
                    self.send_auth_result(Err(Error::Auth("authorization closed unexpectedly".into())));
                }
                let _ = self.emit_disconnect();
                let (lock, cv) = &self.inner.closed;
                *lock.lock().unwrap() = true;
                cv.notify_all();
            }
            _ => {}
        }
    }
}

fn run_chain(client: &Client, update: &Update, chain: &[Middleware], end: &dyn Fn() -> HandlerResult) -> HandlerResult {
    match chain.split_first() {
        None => end(),
        Some((first, rest)) => first(client, update, &|| run_chain(client, update, rest, end)),
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").field("client_id", &self.inner.client_id).finish()
    }
}
