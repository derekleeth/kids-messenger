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
    /// SHA-256 of the file, in hex, so a hub copying it can tell it arrived whole. Missing for a picture whose hub
    /// hasn't worked it out yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
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

// ---- Pairing and talking to each other ------------------------------------------------------------------------

/// A one-time code a parent makes so another hub can join the family's hubs (`POST /hubs/codes`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubCode {
    pub code: String,
    pub expires_at: i64,
}

/// `POST /hubs/pair`: a hub joining with a code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    pub code: String,
    /// The joining hub ([`crate::Hello::hub_id`]).
    pub hub_id: String,
    /// What to call it on the Family screen ("Travel router").
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paired {
    /// The hub that was joined.
    pub hub_id: String,
    pub family_id: String,
    /// The secret the joining hub sends with every request from now on (`Authorization: Hub <its hub ID>:<secret>`).
    /// Sent once; the hub that was joined keeps only its hash.
    pub secret: String,
}

/// `POST /hubs/prove`: before sending anything to an unencrypted address, the joined hub is asked to show it knows
/// the secret, without either side revealing it ([`crate::proof::hub_proof`], keyed by the secret's hash).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubProveRequest {
    /// The hub asking.
    pub hub_id: String,
    pub nonce: String,
}

/// How many rows changed on the hub that took in a batch (`POST /hubs/changes`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub changed: usize,
}

/// One of the family's other hubs, as this hub knows it (`GET /hubs`, for parents).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerInfo {
    pub hub_id: String,
    pub name: String,
    /// Where this hub calls it. `None`: it calls this hub (a travel hub, behind its own network).
    pub url: Option<String>,
    /// When the two last swapped changes. 0: not yet.
    pub last_synced: i64,
}

/// `POST /hubs/files/wanted`: which of these pictures and videos the other hub still needs the file for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesWanted {
    pub ids: Vec<String>,
}

/// The answer to [`FilesWanted`]. IDs in neither list are files the hub already has.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WantedFiles {
    /// Files it needs, with how much of each it already has.
    pub wanted: Vec<FileProgress>,
    /// Pictures it hasn't heard of yet (their records haven't arrived): ask again later.
    pub unknown: Vec<String>,
}

/// How much of a file a hub has (`PUT /hubs/files/{id}` answers with it too).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileProgress {
    pub id: String,
    pub received: u64,
}
