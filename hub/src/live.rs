//! `/live`: a WebSocket each device keeps open, on which the hub pushes changes as they happen.
//!
//! Every change is sent to the broadcast channel with who may see it; each connection passes on the ones meant for
//! its member. A device that was offline, or whose connection fell behind, catches up with `/sync?since=`.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use kit_protocol::Event;
use tokio::sync::broadcast::error::RecvError;

use crate::Hub;
use crate::auth::Authed;
use crate::error::ApiError;
use crate::limits::{LiveSlot, MAX_WS_MESSAGE};

/// Who an event is for.
#[derive(Debug, Clone)]
pub enum Audience {
    Everyone,
    Members(Vec<String>),
    /// Everyone except these members (a list that stopped being shared, for those who aren't its owner).
    AllBut(Vec<String>),
    Device(String),
}

#[derive(Debug)]
pub struct Delivery {
    pub audience: Audience,
    pub event: Event,
}

impl Delivery {
    fn is_for(&self, member_id: &str, device_id: &str) -> bool {
        match &self.audience {
            Audience::Everyone => true,
            Audience::Members(ids) => ids.iter().any(|m| m == member_id),
            Audience::AllBut(ids) => !ids.iter().any(|m| m == member_id),
            Audience::Device(id) => id == device_id,
        }
    }
}

pub fn send(hub: &Hub, audience: Audience, event: Event) {
    // No receivers just means no device is connected right now.
    let _ = hub.live.send(Arc::new(Delivery { audience, event }));
}

const PING_EVERY: Duration = Duration::from_secs(25);

pub async fn handler(State(hub): State<Arc<Hub>>, authed: Authed, ws: WebSocketUpgrade) -> Response {
    let Some(slot) = hub.limits.live_slot(&authed.device_id) else {
        log::warn!("refused a live connection for device {}: too many open", authed.device_id);
        return ApiError::RateLimited(Duration::from_secs(5)).into_response();
    };
    ws.max_message_size(MAX_WS_MESSAGE)
        .max_frame_size(MAX_WS_MESSAGE)
        .on_upgrade(move |socket| run(hub, authed, socket, slot))
}

async fn run(hub: Arc<Hub>, authed: Authed, socket: WebSocket, _slot: LiveSlot) {
    let mut events = hub.live.subscribe();
    let mut stopping = hub.stopping.subscribe();
    let (mut tx, mut rx) = socket.split();
    let mut ping = tokio::time::interval(PING_EVERY);
    log::info!("{} connected (device {})", authed.member.name, authed.device_id);
    loop {
        tokio::select! {
            got = events.recv() => match got {
                Ok(delivery) => {
                    if !delivery.is_for(&authed.member.id, &authed.device_id) {
                        continue;
                    }
                    let Ok(json) = serde_json::to_string(&delivery.event) else { continue };
                    if tx.send(WsMessage::Text(json.into())).await.is_err() {
                        break;
                    }
                    if matches!(delivery.event, Event::Revoked) {
                        let _ = tx.close().await;
                        break;
                    }
                }
                // Fell behind: close, and the device reconnects and syncs what it missed.
                Err(RecvError::Lagged(_)) => break,
                Err(RecvError::Closed) => break,
            },
            incoming = rx.next() => match incoming {
                Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                // Devices send nothing but pings and pongs; changes go through REST.
                Some(Ok(_)) => {}
            },
            // The hub is shutting down: close, so shutdown doesn't wait for devices to hang up.
            _ = stopping.changed() => {
                let _ = tx.send(WsMessage::Close(None)).await;
                break;
            }
            _ = ping.tick() => {
                if tx.send(WsMessage::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }
    log::info!("{} disconnected (device {})", authed.member.name, authed.device_id);
}
