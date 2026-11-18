//! Talking to the family hub: its REST API and the `/live` WebSocket.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use kit_protocol::notes::Note;
use kit_protocol::proof::{HubProof, ProveRequest, hub_proof, token_hash};
use kit_protocol::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream};

use crate::error::{AppError, AppResult};

pub type Live = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// How many addresses a device keeps for its hub.
pub const MAX_ADDRESSES: usize = 5;
/// How long an address gets to prove it's the family's hub, so a dead one doesn't hold up the next.
const PROVE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct HubClient {
    base: String,
    token: Option<String>,
    http: reqwest::Client,
    /// This device has more than one address for the hub, so it may try one that isn't the hub at all.
    several_addresses: bool,
}

/// The hub's address as the person typed it, tidied: `192.168.1.20:8787` → `http://192.168.1.20:8787`.
pub fn normalize_url(raw: &str) -> AppResult<String> {
    let raw = raw.trim().trim_end_matches('/');
    if raw.is_empty() {
        return Err(AppError::Invalid("Type the family hub's address.".into()));
    }
    let url = if raw.contains("://") {
        raw.to_string()
    } else if looks_local(raw) {
        format!("http://{raw}")
    } else {
        format!("https://{raw}")
    };
    let parsed = reqwest::Url::parse(&url).map_err(|_| AppError::Invalid("That address doesn't look right.".into()))?;
    let Some(host) = parsed.host_str() else {
        return Err(AppError::Invalid("The address should start with http:// or https://".into()));
    };
    match parsed.scheme() {
        "https" => {}
        // Plain http sends the device's token unencrypted: only on the home network (or inside Tailscale, which
        // encrypts it), never across the internet.
        "http" if looks_local(host.trim_start_matches('[').trim_end_matches(']')) => {}
        "http" => return Err(AppError::Invalid("A hub outside your home network needs an https:// address.".into())),
        _ => return Err(AppError::Invalid("The address should start with http:// or https://".into())),
    }
    Ok(url)
}

/// A home-network address or name, or one inside a Tailscale network: no TLS needed there.
pub fn looks_local(s: &str) -> bool {
    let host = if s.starts_with('[') { s } else { s.split(['/', ':']).next().unwrap_or_default() };
    let host = host.trim_start_matches('[').split(']').next().unwrap_or_default().to_ascii_lowercase();
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        let [a, b, ..] = ip.octets();
        // 100.64.0.0/10: Tailscale (and other carrier-grade NAT) addresses.
        return ip.is_private() || ip.is_loopback() || ip.is_link_local() || (a == 100 && (64..128).contains(&b));
    }
    if let Ok(ip) = host.parse::<std::net::Ipv6Addr>() {
        return ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local();
    }
    host == "localhost"
        || host.ends_with(".local")
        || host.ends_with(".lan")
        || host.ends_with(".home.arpa")
        || host.ends_with(".ts.net")
        || !host.contains('.')
}

/// A list of hub addresses as typed or scanned: tidied, without repeats, in the same order.
pub fn normalize_urls(raw: &[String]) -> AppResult<Vec<String>> {
    let mut urls: Vec<String> = Vec::new();
    for url in raw.iter().filter(|u| !u.trim().is_empty()) {
        let url = normalize_url(url)?;
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    if urls.is_empty() {
        return Err(AppError::Invalid("Type the family hub's address.".into()));
    }
    if urls.len() > MAX_ADDRESSES {
        return Err(AppError::Invalid(format!("KitChat can keep up to {MAX_ADDRESSES} hub addresses.")));
    }
    Ok(urls)
}

/// Plain http to anywhere but this computer: nothing shows who is really answering.
fn is_unverified(base: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base) else { return true };
    let host = url.host_str().unwrap_or_default().trim_start_matches('[').trim_end_matches(']');
    let on_this_computer =
        host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    url.scheme() == "http" && !on_this_computer
}

/// Tries the addresses in order and returns the first success. Being told this device was removed ends the search;
/// any other failure moves on to the next address. If they all fail, a reason worth showing beats "unreachable".
pub async fn first_working<T, F, Fut>(urls: &[String], mut attempt: F) -> AppResult<T>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = AppResult<T>>,
{
    let mut failure: Option<AppError> = None;
    for url in urls {
        match attempt(url.clone()).await {
            Ok(value) => return Ok(value),
            Err(AppError::Revoked) => return Err(AppError::Revoked),
            Err(e) => {
                log::info!("hub at {url}: {e}");
                if failure.as_ref().is_none_or(|f| matches!(f, AppError::Offline(_))) {
                    failure = Some(e);
                }
            }
        }
    }
    Err(failure.unwrap_or_else(|| AppError::Offline("no hub address".into())))
}

impl HubClient {
    pub fn new(base: &str, token: Option<String>) -> AppResult<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(60))
            .user_agent(concat!("KitChat/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| AppError::Other(format!("http client: {e}")))?;
        Ok(Self { base: normalize_url(base)?, token, http, several_addresses: false })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Says whether this device has more than one address for its hub.
    pub fn with_several_addresses(mut self, several: bool) -> Self {
        self.several_addresses = several;
        self
    }

    /// The same device, talking to the hub at another address.
    pub fn at(&self, base: &str) -> AppResult<Self> {
        Ok(Self::new(base, self.token.clone())?.with_several_addresses(self.several_addresses))
    }

    /// Whether this address has to [`prove`](Self::prove) itself before it's sent the token or believed.
    pub fn needs_proof(&self) -> bool {
        self.several_addresses && is_unverified(&self.base)
    }

    /// Checks this address is the family's hub, without giving anything away: see [`kit_protocol::proof`]. The token
    /// isn't sent, and nothing answered here can sign this device out.
    pub async fn prove(&self, device_id: &str) -> AppResult<()> {
        let token = self.token.as_deref().ok_or(AppError::NotEnrolled)?;
        let mut random = [0u8; 32];
        getrandom::fill(&mut random).map_err(|e| AppError::Other(format!("random: {e}")))?;
        let nonce = proof::hex(&random);
        let request = ProveRequest { device_id: device_id.to_string(), nonce: nonce.clone() };
        let res = self
            .http
            .post(format!("{}/prove", self.base))
            .timeout(PROVE_TIMEOUT)
            .json(&request)
            .send()
            .await
            .map_err(|e| AppError::Offline(e.to_string()))?;
        let status = res.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            return Err(AppError::Offline(format!("prove: {status}")));
        }
        let not_ours = || {
            AppError::Hub(format!(
                "{} didn't confirm it's your family's hub. If it is, make sure the hub is up to date.",
                self.base
            ))
        };
        if !status.is_success() {
            return Err(not_ours());
        }
        let answer: HubProof = res.json().await.map_err(|_| not_ours())?;
        if answer.proof == hub_proof(&token_hash(token), device_id, &nonce) { Ok(()) } else { Err(not_ours()) }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let req = self.http.request(method, format!("{}{path}", self.base));
        match &self.token {
            Some(token) => req.bearer_auth(token),
            None => req,
        }
    }

    async fn send<T: DeserializeOwned>(&self, req: reqwest::RequestBuilder) -> AppResult<T> {
        let res = req.send().await.map_err(|e| AppError::Offline(e.to_string()))?;
        let status = res.status();
        if status == reqwest::StatusCode::UNAUTHORIZED && self.token.is_some() {
            return Err(AppError::Revoked);
        }
        if !status.is_success() {
            let message = res
                .json::<ErrorBody>()
                .await
                .map(|b| b.error)
                .unwrap_or_else(|_| format!("The family hub answered {status}. Is that the right address?"));
            return Err(if status.is_server_error() && status != reqwest::StatusCode::INTERNAL_SERVER_ERROR {
                // A tunnel or proxy in front of a hub that's down.
                AppError::Offline(message)
            } else {
                AppError::Hub(message)
            });
        }
        res.json::<T>().await.map_err(|_| AppError::Hub("That doesn't look like a KitChat hub.".into()))
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> AppResult<T> {
        self.send(self.req(reqwest::Method::GET, path)).await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> AppResult<T> {
        self.send(self.req(reqwest::Method::POST, path).json(body)).await
    }

    pub async fn hello(&self) -> AppResult<Hello> {
        let hello: Hello = self.get("/hello").await?;
        if hello.app != "kitchat-hub" {
            return Err(AppError::Hub("That doesn't look like a KitChat hub.".into()));
        }
        if hello.protocol != PROTOCOL_VERSION {
            return Err(AppError::Hub("This app and the family hub are different versions. Update both.".into()));
        }
        Ok(hello)
    }

    pub async fn setup(&self, req: &SetupRequest) -> AppResult<Enrolled> {
        self.post("/setup", req).await
    }

    pub async fn enroll(&self, req: &EnrollRequest) -> AppResult<Enrolled> {
        self.post("/enroll", req).await
    }

    pub async fn sync(&self, since: i64) -> AppResult<SyncResponse> {
        self.get(&format!("/sync?since={since}")).await
    }

    pub async fn history(&self, chat_id: &str, before: &str) -> AppResult<Vec<Message>> {
        self.get(&format!("/chats/{chat_id}/messages?before={before}&limit=60")).await
    }

    pub async fn send_message(&self, chat_id: &str, req: &SendMessage) -> AppResult<Message> {
        self.post(&format!("/chats/{chat_id}/messages"), req).await
    }

    pub async fn mark_read(&self, chat_id: &str, message_id: &str) -> AppResult<Chat> {
        self.post(&format!("/chats/{chat_id}/read"), &MarkRead { message_id: message_id.into() }).await
    }

    pub async fn react(&self, message_id: &str, emoji: &str) -> AppResult<Message> {
        self.post(&format!("/messages/{message_id}/reactions"), &React { emoji: emoji.into() }).await
    }

    pub async fn direct(&self, member_id: &str) -> AppResult<Chat> {
        self.post("/chats/direct", &serde_json::json!({ "memberId": member_id })).await
    }

    pub async fn upload(&self, bytes: Vec<u8>) -> AppResult<String> {
        let uploaded: Uploaded = self
            .send(
                self.req(reqwest::Method::POST, "/media")
                    .header("Content-Type", "application/octet-stream")
                    .body(bytes),
            )
            .await?;
        Ok(uploaded.media_id)
    }

    /// Starts a big upload (a full-size photo or a video).
    pub async fn start_upload(&self, size: u64) -> AppResult<UploadState> {
        self.post("/media/uploads", &NewUpload { size }).await
    }

    pub async fn upload_state(&self, id: &str) -> AppResult<UploadState> {
        self.get(&format!("/media/uploads/{id}")).await
    }

    pub async fn upload_chunk(&self, id: &str, offset: u64, bytes: Vec<u8>) -> AppResult<UploadState> {
        self.send(
            self.req(reqwest::Method::PUT, &format!("/media/uploads/{id}?offset={offset}"))
                .header("Content-Type", "application/octet-stream")
                // A 4 MB chunk over a slow upload (a phone on mobile data) can take a while.
                .timeout(Duration::from_secs(300))
                .body(bytes),
        )
        .await
    }

    pub async fn finish_upload(&self, id: &str) -> AppResult<String> {
        let req =
            self.req(reqwest::Method::POST, &format!("/media/uploads/{id}/finish")).timeout(Duration::from_secs(300));
        let uploaded: Uploaded = self.send(req).await?;
        Ok(uploaded.media_id)
    }

    /// Downloads a photo or video to `part`, carrying on from whatever is there already. Calls `progress` with the
    /// bytes so far and the total as it goes.
    pub async fn download(&self, id: &str, part: &std::path::Path, progress: impl Fn(u64, u64)) -> AppResult<()> {
        use tokio::io::AsyncWriteExt;
        let have = tokio::fs::metadata(part).await.map(|m| m.len()).unwrap_or(0);
        let mut req = self.req(reqwest::Method::GET, &format!("/media/{id}")).timeout(Duration::from_secs(4 * 60 * 60));
        if have > 0 {
            req = req.header("Range", format!("bytes={have}-"));
        }
        let mut res = req.send().await.map_err(|e| AppError::Offline(e.to_string()))?;
        let (mut file, mut got, total) = match res.status() {
            reqwest::StatusCode::PARTIAL_CONTENT => {
                let file = tokio::fs::OpenOptions::new().append(true).open(part).await?;
                (file, have, have + res.content_length().unwrap_or(0))
            }
            // Nothing left to fetch: the part file is the whole thing.
            reqwest::StatusCode::RANGE_NOT_SATISFIABLE => return Ok(()),
            s if s.is_success() => {
                let file = tokio::fs::File::create(part).await?;
                (file, 0, res.content_length().unwrap_or(0))
            }
            reqwest::StatusCode::UNAUTHORIZED => return Err(AppError::Revoked),
            reqwest::StatusCode::NOT_FOUND => return Err(AppError::Hub("That picture is gone.".into())),
            s => return Err(AppError::Offline(format!("download: {s}"))),
        };
        progress(got, total);
        while let Some(chunk) = res.chunk().await.map_err(|e| AppError::Offline(e.to_string()))? {
            file.write_all(&chunk).await?;
            got += chunk.len() as u64;
            progress(got, total);
        }
        file.flush().await?;
        if total > 0 && got < total {
            return Err(AppError::Offline("the download stopped early".into()));
        }
        Ok(())
    }

    pub async fn save_note(&self, note: &Note) -> AppResult<Note> {
        self.send(self.req(reqwest::Method::PUT, &format!("/notes/{}", note.id)).json(note)).await
    }

    pub async fn delete_note(&self, id: &str) -> AppResult<()> {
        self.send(self.req(reqwest::Method::DELETE, &format!("/notes/{id}"))).await
    }

    pub async fn add_member(&self, req: &NewMember) -> AppResult<Member> {
        self.post("/members", req).await
    }

    pub async fn update_member(&self, id: &str, req: &MemberUpdate) -> AppResult<Member> {
        self.send(self.req(reqwest::Method::PATCH, &format!("/members/{id}")).json(req)).await
    }

    pub async fn join_code(&self, member_id: &str) -> AppResult<EnrollCode> {
        self.send(self.req(reqwest::Method::POST, &format!("/members/{member_id}/codes"))).await
    }

    pub async fn devices(&self) -> AppResult<Vec<Device>> {
        self.get("/devices").await
    }

    pub async fn revoke(&self, device_id: &str) -> AppResult<()> {
        self.send(self.req(reqwest::Method::DELETE, &format!("/devices/{device_id}"))).await
    }

    /// Opens `/live`.
    pub async fn live(&self) -> AppResult<Live> {
        let url = if let Some(rest) = self.base.strip_prefix("https://") {
            format!("wss://{rest}/live")
        } else {
            format!("ws://{}/live", self.base.trim_start_matches("http://"))
        };
        let mut request = url.into_client_request().map_err(|e| AppError::Other(format!("live: {e}")))?;
        if let Some(token) = &self.token {
            let value = format!("Bearer {token}").parse().map_err(|_| AppError::Other("bad token".into()))?;
            request.headers_mut().insert("Authorization", value);
        }
        let connect = tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(tls_connector()));
        match tokio::time::timeout(Duration::from_secs(10), connect).await {
            Err(_) => Err(AppError::Offline("timed out".into())),
            Ok(Ok((stream, _))) => Ok(stream),
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(res))) if res.status() == 401 => Err(AppError::Revoked),
            Ok(Err(e)) => Err(AppError::Offline(e.to_string())),
        }
    }
}

fn tls_connector() -> Connector {
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    Connector::Rustls(Arc::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app installs this at startup (lib.rs); reqwest needs it to make a client.
    fn install_crypto() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    #[test]
    fn addresses_are_tidied() {
        assert_eq!(normalize_url("192.168.1.20:8787/").unwrap(), "http://192.168.1.20:8787");
        assert_eq!(normalize_url("desktop:8787").unwrap(), "http://desktop:8787");
        assert_eq!(normalize_url("kitchat.example.com").unwrap(), "https://kitchat.example.com");
        assert_eq!(normalize_url("http://hub.lan:8787").unwrap(), "http://hub.lan:8787");
        assert!(normalize_url("").is_err());
        assert!(normalize_url("ftp://x.com").is_err());
        assert_eq!(normalize_url("100.101.102.103:8787").unwrap(), "http://100.101.102.103:8787");
        assert_eq!(normalize_url("203.0.113.9:8787").unwrap(), "https://203.0.113.9:8787");
        assert!(normalize_url("http://kitchat.example.com").is_err());
        assert!(normalize_url("http://203.0.113.9").is_err());
        assert_eq!(normalize_url("http://[fd00::1]:8787").unwrap(), "http://[fd00::1]:8787");
    }

    #[test]
    fn address_lists_are_tidied() {
        let raw = ["100.101.102.103:8787", " ", "http://100.101.102.103:8787/", "192.168.1.20:8787"].map(String::from);
        assert_eq!(normalize_urls(&raw).unwrap(), ["http://100.101.102.103:8787", "http://192.168.1.20:8787"]);
        assert!(normalize_urls(&[" ".to_string()]).is_err());
        let many: Vec<String> = (0..=MAX_ADDRESSES).map(|i| format!("192.168.1.{i}:8787")).collect();
        assert!(normalize_urls(&many).is_err());
    }

    #[test]
    fn only_unencrypted_addresses_elsewhere_need_proof_and_only_with_several() {
        install_crypto();
        let client = |url: &str, several| HubClient::new(url, None).unwrap().with_several_addresses(several);
        assert!(client("192.168.1.20:8787", true).needs_proof());
        assert!(client("100.101.102.103:8787", true).needs_proof());
        assert!(client("family-pc:8787", true).needs_proof());
        assert!(!client("192.168.1.20:8787", false).needs_proof());
        assert!(!client("https://chat.example.com", true).needs_proof());
        assert!(!client("localhost:8787", true).needs_proof());
        assert!(!client("127.0.0.1:8787", true).needs_proof());
        assert!(
            client("192.168.1.20:8787", false).with_several_addresses(true).at("10.0.0.2:8787").unwrap().needs_proof()
        );
    }

    fn urls() -> Vec<String> {
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    }

    #[tokio::test]
    async fn failover_uses_the_first_address_that_works() {
        let mut tried = Vec::new();
        let found = first_working(&urls(), |url| {
            tried.push(url.clone());
            async move { if url == "b" { Ok(url) } else { Err(AppError::Offline("down".into())) } }
        })
        .await;
        assert_eq!(found.unwrap(), "b");
        assert_eq!(tried, ["a", "b"]);
    }

    #[tokio::test]
    async fn failover_stops_when_the_device_was_removed() {
        let mut tried = 0;
        let found: AppResult<()> = first_working(&urls(), |_| {
            tried += 1;
            async { Err(AppError::Revoked) }
        })
        .await;
        assert!(matches!(found, Err(AppError::Revoked)));
        assert_eq!(tried, 1);
    }

    #[tokio::test]
    async fn failover_reports_the_most_useful_failure() {
        let found: AppResult<()> = first_working(&urls(), |url| async move {
            Err(if url == "b" { AppError::Hub("update the hub".into()) } else { AppError::Offline("down".into()) })
        })
        .await;
        assert!(matches!(found, Err(AppError::Hub(m)) if m == "update the hub"));
    }

    /// Answers one request with `status` and `body`, and hands back what was asked.
    async fn answer_once(
        status: &'static str,
        body: impl FnOnce(&str) -> String + Send + 'static,
    ) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let served = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                    let length = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().to_string())
                        })
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    if rest.len() >= length {
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&request).into_owned();
            let content = body(text.split_once("\r\n\r\n").unwrap().1);
            let reply = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{content}",
                content.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
            text
        });
        (url, served)
    }

    fn client_at(url: &str) -> HubClient {
        install_crypto();
        HubClient::new(url, Some("secret-token".into())).unwrap().with_several_addresses(true)
    }

    #[tokio::test]
    async fn the_real_hub_passes_and_the_token_stays_home() {
        let (url, served) = answer_once("200 OK", |body| {
            let request: ProveRequest = serde_json::from_str(body).unwrap();
            let proof = hub_proof(&token_hash("secret-token"), &request.device_id, &request.nonce);
            serde_json::to_string(&HubProof { proof }).unwrap()
        })
        .await;
        client_at(&url).prove("device").await.unwrap();
        let request = served.await.unwrap();
        assert!(!request.contains("secret-token"));
        assert!(!request.to_ascii_lowercase().contains("authorization"));
    }

    #[tokio::test]
    async fn a_wrong_answer_is_refused() {
        let (url, _served) = answer_once("200 OK", |_| r#"{"proof":"00"}"#.to_string()).await;
        assert!(matches!(client_at(&url).prove("device").await, Err(AppError::Hub(_))));
    }

    #[tokio::test]
    async fn a_stranger_saying_unauthorized_cannot_sign_the_device_out() {
        let (url, _served) = answer_once("401 Unauthorized", |_| "{}".to_string()).await;
        assert!(matches!(client_at(&url).prove("device").await, Err(AppError::Hub(_))));
    }
}
