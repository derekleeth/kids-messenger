//! Keeping this device in step with the family hub.
//!
//! While the device is part of a family, one background task stays connected: it catches up with `/sync`, then
//! listens on `/live` and applies each change to the local copy, telling the page with a `kit://delta` event. Messages
//! and note edits made here go into the local outbox first and are sent from this task, so nothing typed offline is
//! lost; after a dropped connection it waits a little (longer each time) and starts over.
//!
//! A device can have several addresses for its hub (Tailscale, then the home network). Each connection tries them in
//! order and stays on the first that works until it drops. With several addresses, one may belong to a stranger's
//! machine on another network, so an unencrypted address has to prove it's the family's hub first: until then it gets
//! no token, and nothing it says is believed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use kit_protocol::notes::Note;
use kit_protocol::{Chat, ChatKind, Event, Member, Message, MessageBody, SendMessage, SyncResponse};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::error::{AppError, AppResult};
use crate::hub::{HubClient, Live, first_working};
use crate::platform;
use crate::secret::TokenStore;
use crate::store::{Account, PAGE, Store, UiMessage};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ConnState {
    SignedOut,
    Connecting,
    Online,
    Offline,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: ConnState,
    pub detail: Option<String>,
}

/// What changed, for the page to merge into what it shows. `reset` means "replace everything with this".
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Delta {
    pub reset: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Option<Account>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<Member>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub chats: Vec<Chat>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<UiMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub deleted_notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
}

impl Delta {
    fn is_empty(&self) -> bool {
        !self.reset
            && self.account.is_none()
            && self.members.is_none()
            && self.chats.is_empty()
            && self.messages.is_empty()
            && self.notes.is_empty()
            && self.deleted_notes.is_empty()
            && self.status.is_none()
    }
}

pub struct Engine {
    pub store: Store,
    app: AppHandle,
    tokens: TokenStore,
    pub media_dir: PathBuf,
    client: RwLock<Option<HubClient>>,
    /// The current connection got through to the family's hub at `client`'s address.
    hub_confirmed: AtomicBool,
    /// The ID of the hub the current connection is to (`None` for a hub from before 0.6). Changes from `/live` move
    /// this device's sync position with that hub.
    hub_id: Mutex<Option<String>>,
    kick: Notify,
    /// Bumped when the account changes, so a running session stops and starts again with the new one.
    generation: AtomicU64,
    foreground: AtomicBool,
    status: Mutex<Status>,
    flushing: tokio::sync::Mutex<()>,
    /// Wakes the uploader, which sends photos and videos waiting in the outbox.
    upload_kick: Notify,
    /// Downloads in progress, by media ID, so two parts of the page asking for one video fetch it once.
    downloads: tokio::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// How a photo or video upload is going, for the message's progress ring (`kit://upload`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadProgress {
    pub message_id: String,
    pub sent: u64,
    pub total: u64,
}

/// How a download is going (`kit://download`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub media_id: String,
    pub received: u64,
    pub total: u64,
}

/// A device's saved hub addresses, and which of them couldn't be reached when they were saved.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubAddresses {
    pub urls: Vec<String>,
    pub unreachable: Vec<String>,
}

/// Files made on this device and not sent yet are named `local-<ulid>` in the media folder.
pub fn is_local(media_id: &str) -> bool {
    media_id.starts_with("local-")
}

const FIRST_RETRY: Duration = Duration::from_secs(2);
const LONGEST_RETRY: Duration = Duration::from_secs(60);
/// No message (or ping) from the hub for this long: the connection is taken as dead.
const SILENCE_LIMIT: Duration = Duration::from_secs(70);

impl Engine {
    pub async fn start(app: AppHandle, store: Store, tokens: TokenStore, media_dir: PathBuf) -> Arc<Self> {
        std::fs::create_dir_all(&media_dir).ok();
        let account = store.account().await.ok().flatten();
        let token = match tokens.load() {
            Ok(token) => token,
            Err(e) => {
                log::error!("couldn't read the device token: {e}");
                None
            }
        };
        let client = match (&account, token) {
            (Some(account), Some(token)) => HubClient::new(&account.hub_url, Some(token))
                .ok()
                .map(|client| client.with_several_addresses(account.hub_urls.len() > 1)),
            _ => None,
        };
        let state = if client.is_some() { ConnState::Connecting } else { ConnState::SignedOut };
        let engine = Arc::new(Self {
            store,
            app,
            tokens,
            media_dir,
            client: RwLock::new(client),
            hub_confirmed: AtomicBool::new(false),
            hub_id: Mutex::new(None),
            kick: Notify::new(),
            generation: AtomicU64::new(0),
            foreground: AtomicBool::new(true),
            status: Mutex::new(Status { state, detail: None }),
            flushing: tokio::sync::Mutex::new(()),
            upload_kick: Notify::new(),
            downloads: Default::default(),
        });
        if engine.hub().is_ok() {
            platform::keep_alive(&engine.app, true);
        }
        let runner = engine.clone();
        tauri::async_runtime::spawn(async move { runner.run().await });
        let uploader = engine.clone();
        tauri::async_runtime::spawn(async move { uploader.uploader().await });
        engine
    }

    /// The client for talking to the hub right now. Refuses while an address that has to prove itself hasn't: it could
    /// be anyone's on this network, and this client would send it the device's token.
    pub fn client(&self) -> AppResult<HubClient> {
        let client = self.hub()?;
        if client.needs_proof() && !self.hub_confirmed.load(Ordering::SeqCst) {
            return Err(AppError::Offline("the hub's address isn't confirmed yet".into()));
        }
        Ok(client)
    }

    /// The client as it is, confirmed or not.
    fn hub(&self) -> AppResult<HubClient> {
        self.client.read().unwrap().clone().ok_or(AppError::NotEnrolled)
    }

    /// A different account or set of addresses: anything running on the old one stops.
    fn replace_client(&self, client: Option<HubClient>) {
        let mut slot = self.client.write().unwrap();
        *slot = client;
        self.hub_confirmed.store(false, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Asks the background tasks to send what's waiting now.
    pub fn kick(&self) {
        self.kick.notify_one();
        self.upload_kick.notify_one();
    }

    pub fn set_foreground(&self, foreground: bool) {
        self.foreground.store(foreground, Ordering::Relaxed);
        if foreground {
            self.kick();
        }
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    fn set_status(&self, state: ConnState, detail: Option<String>) {
        let status = Status { state, detail };
        {
            let mut current = self.status.lock().unwrap();
            if current.state == status.state && current.detail == status.detail {
                return;
            }
            *current = status.clone();
        }
        self.emit(Delta { status: Some(status), ..Default::default() });
    }

    pub fn emit(&self, delta: Delta) {
        if delta.is_empty() {
            return;
        }
        if let Err(e) = self.app.emit("kit://delta", &delta) {
            log::warn!("couldn't tell the page: {e}");
        }
    }

    /// Everything the page shows, from the local copy.
    pub async fn snapshot(&self) -> AppResult<Delta> {
        let account = self.store.account().await?;
        let chats = self.store.chats().await?;
        let mut messages = Vec::new();
        for chat in &chats {
            messages.extend(self.store.messages(&chat.id, None, PAGE).await?);
        }
        Ok(Delta {
            reset: true,
            account: Some(account),
            members: Some(self.store.members().await?),
            chats,
            messages,
            notes: self.store.notes().await?,
            deleted_notes: Vec::new(),
            status: Some(self.status()),
        })
    }

    /// This device just joined a family (or set one up): keep the token, start fresh and connect.
    /// `hub_url` is the address that worked, one of `hub_urls`.
    pub async fn enrolled(
        &self,
        hub_url: &str,
        hub_urls: Vec<String>,
        enrolled: kit_protocol::Enrolled,
    ) -> AppResult<()> {
        self.tokens.save(&enrolled.token)?;
        self.store.clear().await?;
        let client = HubClient::new(hub_url, Some(enrolled.token))?.with_several_addresses(hub_urls.len() > 1);
        let account = Account {
            hub_url: hub_url.to_string(),
            hub_urls,
            device_id: enrolled.device_id,
            me: enrolled.member,
            family_name: enrolled.family_name,
        };
        self.store.set_json("account", &account).await?;
        self.replace_client(Some(client));
        *self.status.lock().unwrap() = Status { state: ConnState::Connecting, detail: None };
        platform::keep_alive(&self.app, true);
        self.emit(self.snapshot().await?);
        self.kick();
        Ok(())
    }

    /// Whether the hub answers at this address and knows this device.
    async fn check_address(&self, client: &HubClient, device_id: &str) -> AppResult<()> {
        if client.needs_proof() {
            client.prove(device_id).await?;
        }
        let hello = client.hello().await.map_err(|e| match e {
            AppError::Hub(message) => AppError::Hub(format!("{}: {message}", client.base())),
            other => other,
        })?;
        let rev = self.store.rev(hello.hub_id.as_deref()).await?;
        client.sync(rev).await.map(|_| ())
    }

    /// Replaces this device's hub addresses, in the order to try them. At least one has to work now, so a typing
    /// mistake can't cut the device off; one that can't be reached from here (the home address, while away) is kept and
    /// reported. One that answers but isn't this family's hub is refused.
    pub async fn set_hub_addresses(&self, urls: Vec<String>) -> AppResult<HubAddresses> {
        let mut account = self.store.account().await?.ok_or(AppError::NotEnrolled)?;
        let base = self.hub()?.with_several_addresses(urls.len() > 1);
        let mut working = None;
        let mut unreachable = Vec::new();
        for url in &urls {
            let client = base.at(url)?;
            match self.check_address(&client, &account.device_id).await {
                Ok(()) => working = working.or(Some(client)),
                Err(AppError::Offline(_)) => unreachable.push(url.clone()),
                Err(AppError::Revoked) => {
                    return Err(AppError::Invalid(format!(
                        "The hub at {url} doesn't know this device. Is it your family's hub, with its data copied over?"
                    )));
                }
                Err(e) => return Err(e),
            }
        }
        let Some(client) = working else {
            return Err(AppError::Invalid(
                "Couldn't reach the family hub at any of these addresses. At least one has to work from here.".into(),
            ));
        };
        account.hub_url = client.base().to_string();
        account.hub_urls = urls.clone();
        self.store.set_json("account", &account).await?;
        self.replace_client(Some(client));
        *self.status.lock().unwrap() = Status { state: ConnState::Connecting, detail: None };
        self.emit(Delta { account: Some(Some(account.clone())), status: Some(self.status()), ..Default::default() });
        self.kick();
        log::info!("hub addresses are now {urls:?}");
        Ok(HubAddresses { urls, unreachable })
    }

    /// Forgets the family on this device: signing out, or a parent removed it.
    pub async fn sign_out(&self, why: Option<String>) -> AppResult<()> {
        self.tokens.delete();
        self.replace_client(None);
        self.store.clear().await?;
        if let Ok(entries) = std::fs::read_dir(&self.media_dir) {
            for entry in entries.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        *self.status.lock().unwrap() = Status { state: ConnState::SignedOut, detail: why };
        platform::keep_alive(&self.app, false);
        self.emit(self.snapshot().await?);
        self.kick();
        Ok(())
    }

    async fn run(self: Arc<Self>) {
        let mut retry = FIRST_RETRY;
        loop {
            if self.hub().is_err() {
                self.kick.notified().await;
                continue;
            }
            let generation = self.generation.load(Ordering::SeqCst);
            self.set_status(ConnState::Connecting, None);
            let outcome = match self.connect(generation).await {
                Ok((client, live)) => self.session(&client, live, generation).await,
                Err(e) => Err(e),
            };
            // Wherever this device goes next, that address has to prove itself again.
            if self.generation.load(Ordering::SeqCst) == generation {
                self.hub_confirmed.store(false, Ordering::SeqCst);
            }
            match outcome {
                Ok(()) => retry = FIRST_RETRY,
                Err(AppError::Revoked) => {
                    log::info!("this device was removed from the family");
                    if self.generation.load(Ordering::SeqCst) == generation {
                        let why = "A parent removed this device from the family.".to_string();
                        if let Err(e) = self.sign_out(Some(why)).await {
                            log::error!("couldn't sign out: {e}");
                        }
                    }
                    continue;
                }
                Err(e) => {
                    if self.generation.load(Ordering::SeqCst) != generation {
                        continue;
                    }
                    log::info!("hub connection: {e}");
                    let detail = match &e {
                        AppError::Offline(_) => None,
                        other => Some(other.to_string()),
                    };
                    self.set_status(ConnState::Offline, detail);
                    tokio::select! {
                        _ = tokio::time::sleep(retry) => {}
                        _ = self.kick.notified() => {}
                    }
                    retry = (retry * 2).min(LONGEST_RETRY);
                }
            }
        }
    }

    /// Connects at the first of this device's hub addresses that works, and catches up.
    async fn connect(&self, generation: u64) -> AppResult<(HubClient, Live)> {
        let account = self.store.account().await?.ok_or(AppError::NotEnrolled)?;
        let base = self.hub()?.with_several_addresses(account.hub_urls.len() > 1);
        let (base, device_id) = (&base, account.device_id.as_str());
        first_working(&account.hub_urls, move |url| async move {
            let client = base.at(&url)?;
            let live = self.open(&client, device_id, generation).await?;
            Ok((client, live))
        })
        .await
    }

    /// One address: proves it's the family's hub if it has to, subscribes and catches up.
    async fn open(&self, client: &HubClient, device_id: &str, generation: u64) -> AppResult<Live> {
        if self.generation.load(Ordering::SeqCst) != generation {
            return Err(AppError::Offline("the account changed".into()));
        }
        if client.needs_proof() {
            // Not the hub, or not there: either way, on to the next address. A stranger's answer isn't shown.
            client.prove(device_id).await.map_err(|e| AppError::Offline(e.to_string()))?;
        }
        // Which of the family's hubs this is: the sync position is that hub's.
        let hub_id = client.hello().await?.hub_id;
        let live = client.live().await?;
        // Subscribed first, then caught up: nothing that happens in between is missed.
        let since = self.store.rev(hub_id.as_deref()).await?;
        let sync = client.sync(since).await?;
        if sync.hub_id != hub_id {
            return Err(AppError::Offline("a different hub answered at this address".into()));
        }
        self.use_address(client, generation).await?;
        *self.hub_id.lock().unwrap() = hub_id;
        self.apply_sync(sync, since == 0).await?;
        Ok(live)
    }

    /// From now on everything goes to the hub at this address.
    async fn use_address(&self, client: &HubClient, generation: u64) -> AppResult<()> {
        {
            let mut slot = self.client.write().unwrap();
            if self.generation.load(Ordering::SeqCst) != generation {
                return Err(AppError::Offline("the account changed".into()));
            }
            *slot = Some(client.clone());
            self.hub_confirmed.store(true, Ordering::SeqCst);
        }
        if let Some(mut account) = self.store.account().await?
            && account.hub_url != client.base()
        {
            account.hub_url = client.base().to_string();
            self.store.set_json("account", &account).await?;
            log::info!("now using the hub at {}", account.hub_url);
            self.emit(Delta { account: Some(Some(account)), ..Default::default() });
        }
        // Uploads waiting for the hub can carry on.
        self.upload_kick.notify_one();
        Ok(())
    }

    /// One connection: sends what's waiting, then follows `/live` until it drops or the account changes.
    async fn session(&self, client: &HubClient, mut live: Live, generation: u64) -> AppResult<()> {
        self.set_status(ConnState::Online, None);
        self.flush(client).await?;
        loop {
            tokio::select! {
                msg = tokio::time::timeout(SILENCE_LIMIT, live.next()) => {
                    let msg = match msg {
                        Err(_) => return Err(AppError::Offline("the hub went quiet".into())),
                        Ok(None) => return Err(AppError::Offline("the hub closed the connection".into())),
                        Ok(Some(Err(e))) => return Err(AppError::Offline(e.to_string())),
                        Ok(Some(Ok(msg))) => msg,
                    };
                    match msg {
                        WsMessage::Text(text) => match serde_json::from_str::<Event>(&text) {
                            Ok(event) => self.apply_event(event).await?,
                            Err(e) => log::warn!("unknown event from the hub: {e}"),
                        },
                        WsMessage::Ping(data) => {
                            let _ = live.send(WsMessage::Pong(data)).await;
                        }
                        WsMessage::Close(_) => return Err(AppError::Offline("the hub closed the connection".into())),
                        _ => {}
                    }
                }
                _ = self.kick.notified() => {
                    if self.generation.load(Ordering::SeqCst) != generation {
                        let _ = live.close(None).await;
                        return Ok(());
                    }
                    self.flush(client).await?;
                }
            }
        }
    }

    /// A `/sync` answer. `full` is a first sync with this hub (`since=0`), which has every note this member can see,
    /// so notes this device has that it doesn't have were deleted (or stopped being shared) while it synced elsewhere.
    /// A first sync isn't news, so it doesn't notify.
    async fn apply_sync(&self, sync: SyncResponse, full: bool) -> AppResult<()> {
        let notify = !full;
        let on_hub: std::collections::HashSet<String> =
            if full { sync.notes.iter().map(|n| n.id.clone()).collect() } else { Default::default() };
        let mut delta = Delta::default();
        if let (Some(me), Some(mut account)) = (sync.me.clone(), self.store.account().await?) {
            account.me = me;
            account.family_name = sync.family_name.clone();
            self.store.set_json("account", &account).await?;
            delta.account = Some(Some(account));
        }
        self.store.set_json("members", &sync.members).await?;
        delta.members = Some(sync.members);
        for chat in &sync.chats {
            self.store.put_chat(chat).await?;
        }
        delta.chats = sync.chats;
        let mut fresh = Vec::new();
        for message in sync.messages {
            if self.store.put_message(&message).await? {
                fresh.push(message.clone());
            }
            delta.messages.push(message.into());
        }
        for note in sync.notes {
            if self.store.put_remote_note(&note).await? {
                delta.notes.push(note);
            }
        }
        let mut deleted_notes = sync.deleted_notes;
        if full {
            let gone = self.store.clean_note_ids().await?.into_iter().filter(|id| !on_hub.contains(id));
            deleted_notes.extend(gone);
        }
        for id in &deleted_notes {
            self.store.remove_note(id).await?;
        }
        delta.deleted_notes = deleted_notes;
        self.store.advance_rev(sync.hub_id.as_deref(), sync.rev).await?;
        self.emit(delta);
        if notify {
            self.notify_about(&fresh).await;
        }
        Ok(())
    }

    async fn apply_event(&self, event: Event) -> AppResult<()> {
        let mut delta = Delta::default();
        let rev = match event {
            Event::Message { rev, message } => {
                if self.store.put_message(&message).await? {
                    self.notify_about(std::slice::from_ref(&message)).await;
                }
                delta.messages.push(message.into());
                rev
            }
            Event::Chat { rev, chat } => {
                self.store.put_chat(&chat).await?;
                delta.chats.push(chat);
                rev
            }
            Event::Members { rev, members } => {
                self.store.set_json("members", &members).await?;
                if let Some(mut account) = self.store.account().await?
                    && let Some(me) = members.iter().find(|m| m.id == account.me.id)
                {
                    account.me = me.clone();
                    self.store.set_json("account", &account).await?;
                    delta.account = Some(Some(account));
                }
                delta.members = Some(members);
                rev
            }
            Event::Note { rev, note } => {
                if self.store.put_remote_note(&note).await? {
                    delta.notes.push(note);
                }
                rev
            }
            Event::NoteDeleted { rev, id } => {
                self.store.remove_note(&id).await?;
                delta.deleted_notes.push(id);
                rev
            }
            Event::Revoked => return Err(AppError::Revoked),
        };
        let hub_id = self.hub_id.lock().unwrap().clone();
        self.store.advance_rev(hub_id.as_deref(), rev).await?;
        self.emit(delta);
        Ok(())
    }

    /// Sends the outbox and the notes changed here. Stops at the first sign the hub is unreachable.
    pub async fn flush(&self, client: &HubClient) -> AppResult<()> {
        let _guard = self.flushing.lock().await;
        // A chat's messages go in order: one waiting for its photo or video to upload holds back the ones after it.
        let mut waiting: std::collections::HashSet<String> = Default::default();
        for mut entry in self.store.outbox().await? {
            if entry.error.is_some() || waiting.contains(&entry.chat_id) {
                continue;
            }
            if entry.body.media_ids_mut().iter().any(|id| is_local(id)) {
                waiting.insert(entry.chat_id.clone());
                continue;
            }
            let body = entry.body;
            let send = SendMessage { id: entry.id.clone(), body, sent_at: Some(entry.created_at) };
            match client.send_message(&entry.chat_id, &send).await {
                Ok(message) => {
                    self.store.remove_outbox(&entry.id).await?;
                    self.store.put_message(&message).await?;
                    self.emit(Delta { messages: vec![message.into()], ..Default::default() });
                }
                Err(AppError::Hub(msg)) => self.failed(&entry.id, &msg).await?,
                Err(e) => return Err(e),
            }
        }
        for (note, deleted) in self.store.dirty_notes().await? {
            if deleted {
                match client.delete_note(&note.id).await {
                    Ok(()) | Err(AppError::Hub(_)) => self.store.remove_note(&note.id).await?,
                    Err(e) => return Err(e),
                }
                continue;
            }
            match client.save_note(&note).await {
                Ok(merged) => {
                    self.store.note_saved(&note, &merged).await?;
                    self.emit(Delta { notes: vec![merged], ..Default::default() });
                }
                Err(AppError::Hub(msg)) => {
                    // The hub won't take it (deleted elsewhere, or no longer shared): drop the local copy.
                    log::warn!("note {} refused: {msg}", note.id);
                    self.store.remove_note(&note.id).await?;
                    self.emit(Delta { deleted_notes: vec![note.id.clone()], ..Default::default() });
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Sends the photos and videos of messages in the outbox, one file at a time, then lets `flush` send the
    /// messages. Runs apart from the hub connection, so a long video upload doesn't hold up messages coming in.
    async fn uploader(self: Arc<Self>) {
        loop {
            match self.upload_pending().await {
                Ok(true) => self.kick.notify_one(),
                Ok(false) => {}
                Err(AppError::Revoked | AppError::NotEnrolled) => {}
                Err(e) => {
                    log::info!("upload: {e}");
                    // Offline: try again in a while, or when something changes.
                    let _ = tokio::time::timeout(Duration::from_secs(20), self.upload_kick.notified()).await;
                    continue;
                }
            }
            self.upload_kick.notified().await;
        }
    }

    /// Uploads what's waiting. Says whether a message became ready to send.
    async fn upload_pending(&self) -> AppResult<bool> {
        let mut ready = false;
        for entry in self.store.outbox().await? {
            if entry.error.is_some() {
                continue;
            }
            let mut body = entry.body.clone();
            let locals: Vec<String> =
                body.media_ids_mut().into_iter().filter(|id| is_local(id)).map(|id| id.clone()).collect();
            if locals.is_empty() {
                continue;
            }
            let client = self.client()?;
            let mut total = 0;
            for local in &locals {
                total += tokio::fs::metadata(self.media_dir.join(local)).await.map(|m| m.len()).unwrap_or(0);
            }
            let mut done = 0;
            let mut failed = false;
            for local in locals {
                let path = self.media_dir.join(&local);
                let media_id = match self.upload_file(&client, &entry.id, &path, &local, done, total).await {
                    Ok(id) => id,
                    Err(AppError::Hub(msg)) => {
                        self.failed(&entry.id, &msg).await?;
                        failed = true;
                        break;
                    }
                    Err(AppError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                        self.failed(&entry.id, "The picture went missing before it was sent.").await?;
                        failed = true;
                        break;
                    }
                    Err(e) => return Err(e),
                };
                done += tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
                let _ = tokio::fs::rename(&path, self.media_dir.join(&media_id)).await;
                for id in body.media_ids_mut() {
                    if *id == local {
                        *id = media_id.clone();
                    }
                }
                self.store.uploaded(&entry.id, &body).await?;
                self.store.set(&format!("upload:{local}"), "").await?;
            }
            if !failed {
                ready = true;
            }
        }
        Ok(ready)
    }

    /// One file: in one request when it's a small picture, in chunks (resumed if a try was cut off) otherwise.
    async fn upload_file(
        &self,
        client: &HubClient,
        message_id: &str,
        path: &std::path::Path,
        local: &str,
        done: u64,
        total: u64,
    ) -> AppResult<String> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let progress = |sent: u64| {
            let p = UploadProgress { message_id: message_id.to_string(), sent: done + sent, total };
            let _ = self.app.emit("kit://upload", p);
        };
        let size = tokio::fs::metadata(path).await?.len();
        let mut file = tokio::fs::File::open(path).await?;
        let mut head = [0u8; 16];
        let n = file.read(&mut head).await?;
        let kind = kit_protocol::media::sniff(&head[..n]).unwrap_or_default();
        if size <= 2 * 1024 * 1024 && kit_protocol::media::is_shown_picture(kind) {
            let bytes = tokio::fs::read(path).await?;
            let id = client.upload(bytes).await?;
            progress(size);
            return Ok(id);
        }
        let key = format!("upload:{local}");
        let resumed = match self.store.get(&key).await?.filter(|id| !id.is_empty()) {
            Some(id) => match client.upload_state(&id).await {
                Ok(state) if state.size == size => Some(state),
                Ok(_) | Err(AppError::Hub(_)) => None,
                Err(e) => return Err(e),
            },
            None => None,
        };
        let mut state = match resumed {
            Some(state) => state,
            None => {
                let started = client.start_upload(size).await?;
                self.store.set(&key, &started.upload_id).await?;
                started
            }
        };
        let mut chunk = vec![0u8; kit_protocol::UPLOAD_CHUNK_BYTES];
        while state.received < size {
            progress(state.received);
            file.seek(std::io::SeekFrom::Start(state.received)).await?;
            let want = ((size - state.received) as usize).min(chunk.len());
            file.read_exact(&mut chunk[..want]).await?;
            state = match client.upload_chunk(&state.upload_id, state.received, chunk[..want].to_vec()).await {
                Ok(state) => state,
                // The hub has a different amount than this device thought (a chunk whose answer was lost): ask it.
                Err(AppError::Hub(_)) => client.upload_state(&state.upload_id).await?,
                Err(e) => return Err(e),
            };
        }
        progress(size);
        client.finish_upload(&state.upload_id).await
    }

    /// Where a photo or video is on this device, downloading it from the hub first when it isn't here yet.
    pub async fn media_file(&self, id: &str) -> AppResult<PathBuf> {
        let path = self.media_dir.join(id);
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(path);
        }
        if is_local(id) {
            return Err(AppError::Invalid("That picture isn't on this device anymore.".into()));
        }
        let lock = self.downloads.lock().await.entry(id.to_string()).or_default().clone();
        let _held = lock.lock().await;
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(path);
        }
        let part = self.media_dir.join(format!("{id}.part"));
        let progress = |received: u64, total: u64| {
            let _ = self.app.emit("kit://download", DownloadProgress { media_id: id.to_string(), received, total });
        };
        let result = self.client()?.download(id, &part, progress).await;
        self.downloads.lock().await.remove(id);
        result?;
        tokio::fs::rename(&part, &path).await?;
        Ok(path)
    }

    async fn failed(&self, id: &str, msg: &str) -> AppResult<()> {
        self.store.outbox_failed(id, Some(msg)).await?;
        if let Some(entry) = self.store.outbox_entry(id).await? {
            let me = self.store.account().await?.map(|a| a.me.id).unwrap_or_default();
            self.emit(Delta { messages: vec![crate::store::outbox_message(&entry, &me)], ..Default::default() });
        }
        Ok(())
    }

    /// A notification for messages from someone else, while the app isn't on screen.
    async fn notify_about(&self, messages: &[Message]) {
        if self.foreground.load(Ordering::Relaxed) {
            return;
        }
        let Ok(Some(account)) = self.store.account().await else { return };
        let incoming: Vec<&Message> = messages.iter().filter(|m| m.sender_id != account.me.id).collect();
        let Some(last) = incoming.last() else { return };
        let members = self.store.members().await.unwrap_or_default();
        let name = |id: &str| members.iter().find(|m| m.id == id).map(|m| m.name.clone()).unwrap_or("Someone".into());
        let in_family = self
            .store
            .chats()
            .await
            .unwrap_or_default()
            .iter()
            .any(|c| c.id == last.chat_id && c.kind == ChatKind::Family);
        let title = if incoming.len() > 1 {
            format!("{} new messages", incoming.len())
        } else if in_family {
            format!("{} · Family chat", name(&last.sender_id))
        } else {
            name(&last.sender_id)
        };
        let body = match &last.body {
            MessageBody::Text { text } => text.chars().take(140).collect(),
            MessageBody::Image { caption, .. } if !caption.is_empty() => format!("📷 {caption}"),
            MessageBody::Image { .. } => "📷 Sent a picture".to_string(),
            MessageBody::Video { caption, .. } if !caption.is_empty() => format!("🎬 {caption}"),
            MessageBody::Video { .. } => "🎬 Sent a video".to_string(),
            MessageBody::Sticker { .. } => "Sent a sticker".to_string(),
            MessageBody::Note { title, .. } => format!("📝 Shared \"{title}\""),
        };
        let body = if incoming.len() > 1 { format!("{}: {body}", name(&last.sender_id)) } else { body };
        platform::notify(&self.app, &title, &body);
        // The page opens this chat when the window next comes forward (a notification's click doesn't reach us).
        let _ = self.app.emit("kit://notified", &last.chat_id);
    }
}
