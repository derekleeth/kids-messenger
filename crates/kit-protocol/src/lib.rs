//! What the KitChat app and the family hub send each other, over REST (JSON) and the `/live` WebSocket.
//!
//! Field names are camelCase on the wire, so the React side can use them as they are. Times are milliseconds since the
//! Unix epoch. IDs are ULIDs; a message's ID is chosen by the sending device, so a resent message (the app's outbox
//! after a dropped connection) is stored once.

use serde::{Deserialize, Serialize};

pub mod hubs;
pub mod links;
pub mod media;
pub mod notes;
pub mod proof;

/// The protocol version the hub reports in `/hello`; an app refuses a hub with a different major version.
pub const PROTOCOL_VERSION: u32 = 1;

/// Largest file the hub takes in one request (`POST /media`, or one chunk of a bigger upload), in bytes. The small
/// picture each photo and video is shown with in the chat is well below it.
pub const MAX_MEDIA_BYTES: usize = 15 * 1024 * 1024;

/// Largest full-size photo or video, sent in chunks (`/media/uploads`), in bytes.
pub const MAX_ORIGINAL_BYTES: u64 = 1024 * 1024 * 1024;

/// The size of each chunk of a big upload. Well under Cloudflare's 100 MB limit on one request.
pub const UPLOAD_CHUNK_BYTES: usize = 4 * 1024 * 1024;

/// Longest text message, in characters.
pub const MAX_TEXT_CHARS: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    /// Manages the family: adds members and devices, and removes devices.
    Parent,
    Kid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub id: String,
    pub name: String,
    pub role: Role,
    /// A colour name from the app's palette ("sun", "sky", "bubblegum", "mint", "grape", "tangerine").
    pub color: String,
    /// An emoji or Kit pose shown as the member's picture.
    pub avatar: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub member_id: String,
    pub name: String,
    pub created_at: i64,
    pub last_seen: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChatKind {
    /// The whole family. There is exactly one.
    Family,
    /// Two members.
    Direct,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chat {
    pub id: String,
    pub kind: ChatKind,
    pub member_ids: Vec<String>,
    /// The newest message each member has seen, by member ID.
    pub read_up_to: std::collections::BTreeMap<String, String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum MessageBody {
    Text {
        text: String,
    },
    Image {
        /// The picture the chat shows: at most 1920 pixels on its longest side.
        media_id: String,
        width: u32,
        height: u32,
        #[serde(default)]
        caption: String,
        /// The photo at its full size, opened when it's tapped. `None` for pictures sent before 0.2, and for types
        /// whose hidden data the app can't take out (HEIC).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        full_media_id: Option<String>,
        #[serde(default, skip_serializing_if = "is_zero")]
        full_bytes: u64,
    },
    Video {
        video_media_id: String,
        /// A still from the video (a JPEG), shown until it's played.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        poster_media_id: Option<String>,
        width: u32,
        height: u32,
        duration_ms: u64,
        bytes: u64,
        #[serde(default)]
        caption: String,
    },
    /// One of Kit's stickers, by name ("wave", "love", "lol", …).
    Sticker {
        sticker: String,
    },
    /// A note or list shared into the chat: a snapshot of it when it was shared.
    Note {
        note_id: String,
        title: String,
        emoji: String,
        /// The first few lines, for the card.
        preview: Vec<String>,
        /// A family list everyone can open and edit; otherwise the card is all the others see.
        shared: bool,
    },
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl MessageBody {
    /// The media IDs this message points at.
    pub fn media_ids_mut(&mut self) -> Vec<&mut String> {
        match self {
            MessageBody::Image { media_id, full_media_id, .. } => {
                let mut ids = vec![media_id];
                ids.extend(full_media_id.as_mut());
                ids
            }
            MessageBody::Video { video_media_id, poster_media_id, .. } => {
                let mut ids: Vec<&mut String> = poster_media_id.as_mut().into_iter().collect();
                ids.push(video_media_id);
                ids
            }
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reaction {
    pub member_id: String,
    pub emoji: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkPreview {
    pub url: String,
    pub title: String,
    pub description: String,
    pub site: String,
    /// The page's picture, copied to the hub so devices never load it from the site.
    pub image_media_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub chat_id: String,
    pub sender_id: String,
    pub body: MessageBody,
    pub created_at: i64,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    /// Filled in by the hub a moment after a message with a link arrives.
    #[serde(default)]
    pub link: Option<LinkPreview>,
    /// The change counter when this message last changed (sent, reacted to, preview added).
    #[serde(default)]
    pub rev: i64,
}

// ---- Requests and responses -------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub app: String,
    pub protocol: u32,
    pub family_name: String,
    /// True until the first parent has set the family up.
    pub needs_setup: bool,
    /// This hub, out of the family's hubs: a device keeps its sync position (`rev`) for each hub separately, since
    /// every hub counts its changes on its own. `None` from hubs before 0.6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_id: Option<String>,
    /// The family, the same on every hub it has. `None` until the family is set up, and from hubs before 0.6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_id: Option<String>,
}

/// The first parent sets up the family with the code the hub printed when it started.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupRequest {
    pub setup_code: String,
    pub family_name: String,
    pub name: String,
    pub device_name: String,
}

/// A new device joins with a code a parent made for a member.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollRequest {
    pub code: String,
    pub device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Enrolled {
    /// The device's secret. Sent once; the hub keeps only its hash.
    pub token: String,
    pub device_id: String,
    pub member: Member,
    pub family_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMember {
    pub name: String,
    pub role: Role,
    pub color: String,
    pub avatar: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberUpdate {
    pub name: Option<String>,
    pub color: Option<String>,
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollCode {
    pub code: String,
    pub member_id: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessage {
    /// Chosen by the device (a ULID), so a resend is stored once.
    pub id: String,
    pub body: MessageBody,
    /// When it was written, for a message the device sends later (from its outbox, after being offline). The hub
    /// keeps it as the message's time if it's believable ([`sent_at_to_keep`]), so the message sorts where it was
    /// written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<i64>,
}

/// How far back a device's `sentAt` is believed: a week, about as long as an outbox would hold a message.
pub const SENT_AT_MAX_AGE_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// How far a `sentAt` may be ahead of the hub's clock, or away from the time in the message's own ULID.
pub const SENT_AT_SLACK_MS: i64 = 10 * 60 * 1000;

/// The time to keep for message `id`, sent with `sent_at` and arriving at `now`: `sent_at` if it isn't in the future,
/// isn't more than [`SENT_AT_MAX_AGE_MS`] old, and is close to the ULID's own time (the device makes both when the
/// message is written); otherwise `now`.
pub fn sent_at_to_keep(id: &str, sent_at: Option<i64>, now: i64) -> i64 {
    let Some(sent_at) = sent_at else { return now };
    let Ok(ulid) = ulid::Ulid::from_string(id) else { return now };
    let id_time = ulid.timestamp_ms() as i64;
    let believable = sent_at <= now + SENT_AT_SLACK_MS
        && sent_at >= now - SENT_AT_MAX_AGE_MS
        && (sent_at - id_time).abs() <= SENT_AT_SLACK_MS;
    if believable { sent_at.min(now) } else { now }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct React {
    /// An emoji, or empty to take this member's reaction away.
    pub emoji: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkRead {
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Uploaded {
    pub media_id: String,
}

/// Starts a big upload (`POST /media/uploads`), sent afterwards in chunks of `UPLOAD_CHUNK_BYTES`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewUpload {
    pub size: u64,
}

/// A big upload in progress: how much of it the hub has, so a device that lost its connection carries on from there.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadState {
    pub upload_id: String,
    pub size: u64,
    pub received: u64,
}

/// Everything that changed since a device last synced. `since = 0` is a first sync: all members and chats, the newest
/// messages of each chat, and every note the device's member can see.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResponse {
    pub rev: i64,
    /// The hub `rev` belongs to ([`Hello::hub_id`]). `None` from hubs before 0.6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_id: Option<String>,
    pub me: Option<Member>,
    pub family_name: String,
    pub members: Vec<Member>,
    pub chats: Vec<Chat>,
    pub messages: Vec<Message>,
    pub notes: Vec<notes::Note>,
    pub deleted_notes: Vec<String>,
}

/// What `/live` pushes. Each carries the change counter it brought the hub to, so a device that reconnects asks
/// `/sync?since=` its last one and misses nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type")]
pub enum Event {
    /// A new message, or one that changed (a reaction, a link preview).
    Message {
        rev: i64,
        message: Message,
    },
    Chat {
        rev: i64,
        chat: Chat,
    },
    Members {
        rev: i64,
        members: Vec<Member>,
    },
    Note {
        rev: i64,
        note: notes::Note,
    },
    NoteDeleted {
        rev: i64,
        id: String,
    },
    /// This device was removed by a parent; it should forget its token.
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    pub error: String,
}

/// The colours a member or a note can have; the app has a shade of each for light and dark themes.
pub const COLORS: &[&str] = &["sun", "sky", "bubblegum", "mint", "grape", "tangerine"];

/// Kit's stickers. The app has a picture for each; the hub only checks the name.
pub const STICKERS: &[&str] = &["wave", "love", "lol", "wow", "sleepy", "party", "thinking", "thumbsup", "hug", "cool"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_body_is_tagged_camel_case() {
        let body = MessageBody::Image {
            media_id: "m1".into(),
            width: 10,
            height: 20,
            caption: String::new(),
            full_media_id: None,
            full_bytes: 0,
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["kind"], "image");
        assert_eq!(json["mediaId"], "m1");
        assert!(json.get("fullMediaId").is_none());
        let back: MessageBody = serde_json::from_value(json).unwrap();
        assert_eq!(back, body);
    }

    #[test]
    fn videos_round_trip() {
        let body = MessageBody::Video {
            video_media_id: "v".into(),
            poster_media_id: Some("p".into()),
            width: 1080,
            height: 1920,
            duration_ms: 4200,
            bytes: 9_000_000,
            caption: "hi".into(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["kind"], "video");
        assert_eq!(json["durationMs"], 4200);
        assert_eq!(serde_json::from_value::<MessageBody>(json).unwrap(), body);
    }

    #[test]
    fn events_round_trip() {
        let event = Event::NoteDeleted { rev: 4, id: "n".into() };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"noteDeleted\""));
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
    }

    #[test]
    fn a_believable_sent_at_is_kept_and_anything_else_is_now() {
        let now = 1_800_000_000_000;
        let written = now - 6 * 60 * 60 * 1000;
        let id = ulid::Ulid::from_parts(written as u64, 7).to_string();
        assert_eq!(sent_at_to_keep(&id, Some(written), now), written);
        assert_eq!(sent_at_to_keep(&id, None, now), now);
        // Far from the ULID's own time.
        assert_eq!(sent_at_to_keep(&id, Some(written - 60 * 60 * 1000), now), now);
        // Older than a week.
        let old = now - SENT_AT_MAX_AGE_MS - 1;
        assert_eq!(sent_at_to_keep(&ulid::Ulid::from_parts(old as u64, 7).to_string(), Some(old), now), now);
        // A little ahead of the hub's clock is taken as now; far ahead isn't believed.
        let ahead = now + 60 * 1000;
        assert_eq!(sent_at_to_keep(&ulid::Ulid::from_parts(ahead as u64, 7).to_string(), Some(ahead), now), now);
        let far = now + SENT_AT_SLACK_MS + 1;
        assert_eq!(sent_at_to_keep(&ulid::Ulid::from_parts(far as u64, 7).to_string(), Some(far), now), now);
        assert_eq!(sent_at_to_keep("not-a-ulid", Some(written), now), now);
    }

    #[test]
    fn caption_defaults_to_empty() {
        let body: MessageBody = serde_json::from_str(r#"{"kind":"image","mediaId":"m","width":1,"height":1}"#).unwrap();
        assert!(matches!(body, MessageBody::Image { caption, .. } if caption.is_empty()));
    }
}
