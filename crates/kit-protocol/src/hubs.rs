//! What a family's hubs send each other to keep in step (docs/multi_hub_plan.md).
//!
//! Unlike `/sync`, which is what one member may see, a hub gets everything, with what it needs to combine two copies:
//! when members and reactions last changed, reactions that were taken away, removed devices, and the hashes of the
//! devices' tokens (so a device can sign in on any of the family's hubs).

use serde::{Deserialize, Serialize};

use crate::notes::Note;
use crate::{ChatKind, LinkPreview, Member, MessageBody};

/// One batch of changes from a hub: everything that changed on it after the number asked for, up to `rev`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    /// The hub these come from.
    pub hub_id: String,
    pub family: FamilyRecord,
    /// The sending hub's change number this batch goes up to: ask for what's after it next time.
    pub rev: i64,
    /// There's more after `rev` already: ask again straight away.
    pub more: bool,
    /// Every member, whatever changed: the other records point at them, and a family is small.
    pub members: Vec<MemberRecord>,
    pub devices: Vec<DeviceRecord>,
    /// The chats that changed, and the ones the messages in this batch are in.
    pub chats: Vec<ChatRecord>,
    pub messages: Vec<MessageRecord>,
    pub notes: Vec<NoteRecord>,
    pub media: Vec<MediaRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyRecord {
    pub family_id: String,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberRecord {
    #[serde(flatten)]
    pub member: Member,
    pub created_at: i64,
    /// When the name, colour or picture last changed: the newer of two copies wins.
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRecord {
    pub id: String,
    pub member_id: String,
    pub name: String,
    pub token_hash: String,
    pub created_at: i64,
    pub last_seen: i64,
    /// Removed by a parent. Removed on one hub is removed on all.
    pub revoked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRecord {
    pub id: String,
    pub kind: ChatKind,
    /// A 1:1 chat's two member IDs, sorted and joined.
    pub direct_key: Option<String>,
    pub created_at: i64,
    pub members: Vec<ChatMemberRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMemberRecord {
    pub member_id: String,
    /// The newest message this member has seen here. The later of two copies wins (ULIDs sort by time).
    pub read_up_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRecord {
    pub id: String,
    pub chat_id: String,
    pub sender_id: String,
    pub body: MessageBody,
    pub link: Option<LinkPreview>,
    pub created_at: i64,
    /// Taken-away ones too, with an empty `emoji`.
    pub reactions: Vec<ReactionRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionRecord {
    pub member_id: String,
    /// Empty: taken away.
    pub emoji: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteRecord {
    pub note: Note,
    pub deleted: bool,
}

/// A photo or video's record. The file itself is copied separately.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaRecord {
    pub id: String,
    pub uploader_id: Option<String>,
    pub content_type: String,
    pub size: u64,
    pub created_at: i64,
}

/// Whether `a` is the newer of two versions of something. Equal times are settled by content, so every hub picks the
/// same one whichever copy it had first.
pub fn newer<T: Serialize>(a_time: i64, a: &T, b_time: i64, b: &T) -> bool {
    if a_time != b_time {
        return a_time > b_time;
    }
    let a = serde_json::to_string(a).unwrap_or_default();
    let b = serde_json::to_string(b).unwrap_or_default();
    a > b
}
