//! What the KitChat app and the family hub send each other, over REST (JSON) and the `/live` WebSocket.
//!
//! Field names are camelCase on the wire, so the React side can use them as they are. Times are milliseconds since the
//! Unix epoch. IDs are ULIDs; a message's ID is chosen by the sending device, so a resent message (the app's outbox
//! after a dropped connection) is stored once.

use serde::{Deserialize, Serialize};

pub mod links;
pub mod notes;

/// The protocol version the hub reports in `/hello`; an app refuses a hub with a different major version.
pub const PROTOCOL_VERSION: u32 = 1;

/// Largest picture the hub accepts, in bytes. The app shrinks pictures well below this before sending.
pub const MAX_MEDIA_BYTES: usize = 15 * 1024 * 1024;

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
        media_id: String,
        width: u32,
        height: u32,
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

/// Everything that changed since a device last synced. `since = 0` is a first sync: all members and chats, the newest
/// messages of each chat, and every note the device's member can see.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResponse {
    pub rev: i64,
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
        let body = MessageBody::Image { media_id: "m1".into(), width: 10, height: 20, caption: String::new() };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["kind"], "image");
        assert_eq!(json["mediaId"], "m1");
        let back: MessageBody = serde_json::from_value(json).unwrap();
        assert_eq!(back, body);
    }

    #[test]
    fn events_round_trip() {
        let event = Event::NoteDeleted { rev: 4, id: "n".into() };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"noteDeleted\""));
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
    }

    #[test]
    fn caption_defaults_to_empty() {
        let body: MessageBody = serde_json::from_str(r#"{"kind":"image","mediaId":"m","width":1,"height":1}"#).unwrap();
        assert!(matches!(body, MessageBody::Image { caption, .. } if caption.is_empty()));
    }
}
