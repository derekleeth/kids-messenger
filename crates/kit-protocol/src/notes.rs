//! Notes and lists, and how two edits of the same one are combined.
//!
//! A note is a title plus blocks (a heading, a line of text, a checklist item, a bullet, a link). Family lists can be
//! edited by several people at once, so the hub never takes a whole note over another: each block, and the note's own
//! fields (title, emoji, colour, shared), keep whichever edit is newest (`updatedAt`). Deleting a block leaves a
//! tombstone (`deleted: true`), so a device that hasn't heard of the deletion can't bring the block back.

use serde::{Deserialize, Serialize};

/// Most blocks in one note, tombstones included.
pub const MAX_BLOCKS: usize = 500;
/// Longest block text, in characters.
pub const MAX_BLOCK_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BlockKind {
    Heading,
    Text,
    Check,
    Bullet,
    Link,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub id: String,
    pub kind: BlockKind,
    #[serde(default)]
    pub text: String,
    /// Ticked, for a checklist item.
    #[serde(default)]
    pub checked: bool,
    /// The address, for a link.
    #[serde(default)]
    pub url: String,
    /// Sort position. A block moved between two others gets a number between theirs.
    pub pos: f64,
    pub updated_at: i64,
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub owner_id: String,
    pub title: String,
    pub emoji: String,
    pub color: String,
    /// A family list: every member can see and edit it. Only the owner can change this.
    pub shared: bool,
    pub blocks: Vec<Block>,
    pub created_at: i64,
    /// When the title, emoji, colour or `shared` last changed.
    pub meta_updated_at: i64,
    #[serde(default)]
    pub rev: i64,
}

impl Note {
    /// The newest edit anywhere in the note.
    pub fn updated_at(&self) -> i64 {
        self.blocks.iter().map(|b| b.updated_at).fold(self.meta_updated_at, i64::max)
    }

    /// The blocks to show, in order.
    pub fn visible_blocks(&self) -> Vec<&Block> {
        let mut blocks: Vec<&Block> = self.blocks.iter().filter(|b| !b.deleted).collect();
        blocks.sort_by(|a, b| a.pos.total_cmp(&b.pos).then_with(|| a.id.cmp(&b.id)));
        blocks
    }

    /// The first few lines, for a card in a chat.
    pub fn preview(&self, lines: usize) -> Vec<String> {
        self.visible_blocks()
            .into_iter()
            .map(|b| match b.kind {
                BlockKind::Check => format!("{} {}", if b.checked { "☑" } else { "☐" }, b.text),
                BlockKind::Bullet => format!("• {}", b.text),
                BlockKind::Link if b.text.is_empty() => b.url.clone(),
                _ => b.text.clone(),
            })
            .filter(|line| !line.trim().is_empty())
            .take(lines)
            .collect()
    }
}

/// Whether `a` is the newer of two versions. Equal times are settled by content, so every device picks the same one.
fn newer<T: Serialize>(a_time: i64, a: &T, b_time: i64, b: &T) -> bool {
    if a_time != b_time {
        return a_time > b_time;
    }
    let a = serde_json::to_string(a).unwrap_or_default();
    let b = serde_json::to_string(b).unwrap_or_default();
    a > b
}

#[derive(Serialize)]
struct Meta<'a> {
    title: &'a str,
    emoji: &'a str,
    color: &'a str,
    shared: bool,
}

/// Combines a stored note with an incoming edit of it. `may_change_sharing` is false when the editor isn't the
/// owner: their version of `shared` is ignored.
pub fn merge(stored: &Note, incoming: &Note, may_change_sharing: bool) -> Note {
    let mut out = stored.clone();

    let stored_meta = Meta { title: &stored.title, emoji: &stored.emoji, color: &stored.color, shared: stored.shared };
    let shared = if may_change_sharing { incoming.shared } else { stored.shared };
    let incoming_meta = Meta { title: &incoming.title, emoji: &incoming.emoji, color: &incoming.color, shared };
    if newer(incoming.meta_updated_at, &incoming_meta, stored.meta_updated_at, &stored_meta) {
        out.title = incoming.title.clone();
        out.emoji = incoming.emoji.clone();
        out.color = incoming.color.clone();
        out.shared = shared;
        out.meta_updated_at = incoming.meta_updated_at;
    }

    for block in &incoming.blocks {
        match out.blocks.iter_mut().find(|b| b.id == block.id) {
            Some(existing) => {
                if newer(block.updated_at, block, existing.updated_at, existing) {
                    *existing = block.clone();
                }
            }
            None => out.blocks.push(block.clone()),
        }
    }
    out
}

/// Checks an incoming note before it's stored. Returns what's wrong, for the error message.
pub fn validate(note: &Note) -> Result<(), String> {
    if note.id.is_empty() || note.id.len() > 64 {
        return Err("a note needs an ID".into());
    }
    if note.title.chars().count() > 200 {
        return Err("that title is too long".into());
    }
    if note.blocks.len() > MAX_BLOCKS {
        return Err(format!("a note can have up to {MAX_BLOCKS} lines"));
    }
    for block in &note.blocks {
        if block.text.chars().count() > MAX_BLOCK_CHARS || block.url.len() > MAX_BLOCK_CHARS {
            return Err("one of the lines is too long".into());
        }
        if !block.pos.is_finite() {
            return Err("a line has a bad position".into());
        }
        if block.kind == BlockKind::Link && !block.url.is_empty() && crate::links::normalize(&block.url).is_none() {
            return Err("links must start with http:// or https://".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(id: &str, text: &str, at: i64) -> Block {
        Block {
            id: id.into(),
            kind: BlockKind::Check,
            text: text.into(),
            checked: false,
            url: String::new(),
            pos: 1.0,
            updated_at: at,
            deleted: false,
        }
    }

    fn note(blocks: Vec<Block>) -> Note {
        Note {
            id: "n1".into(),
            owner_id: "kid".into(),
            title: "Cool sites".into(),
            emoji: "🌐".into(),
            color: "sky".into(),
            shared: true,
            blocks,
            created_at: 1,
            meta_updated_at: 1,
            rev: 0,
        }
    }

    #[test]
    fn edits_to_different_blocks_both_survive() {
        let base = note(vec![block("a", "apples", 1), block("b", "bananas", 1)]);
        let mut mine = base.clone();
        mine.blocks[0].text = "green apples".into();
        mine.blocks[0].updated_at = 5;
        let mut theirs = base.clone();
        theirs.blocks[1].checked = true;
        theirs.blocks[1].updated_at = 6;

        let merged = merge(&merge(&base, &mine, true), &theirs, true);
        assert_eq!(merged.blocks[0].text, "green apples");
        assert!(merged.blocks[1].checked);
    }

    #[test]
    fn an_older_edit_does_not_win() {
        let stored = note(vec![block("a", "new", 10)]);
        let incoming = note(vec![block("a", "old", 3)]);
        assert_eq!(merge(&stored, &incoming, true).blocks[0].text, "new");
    }

    #[test]
    fn a_deletion_is_not_undone_by_a_stale_copy() {
        let mut deleted = block("a", "x", 9);
        deleted.deleted = true;
        let stored = note(vec![deleted]);
        let stale = note(vec![block("a", "x", 2)]);
        let merged = merge(&stored, &stale, true);
        assert!(merged.blocks[0].deleted);
        assert!(merged.visible_blocks().is_empty());
    }

    #[test]
    fn new_blocks_are_added() {
        let stored = note(vec![block("a", "x", 1)]);
        let incoming = note(vec![block("a", "x", 1), block("b", "y", 2)]);
        assert_eq!(merge(&stored, &incoming, true).blocks.len(), 2);
    }

    #[test]
    fn only_the_owner_changes_sharing() {
        let stored = note(vec![]);
        let mut incoming = stored.clone();
        incoming.shared = false;
        incoming.title = "Renamed".into();
        incoming.meta_updated_at = 5;
        let merged = merge(&stored, &incoming, false);
        assert!(merged.shared);
        assert_eq!(merged.title, "Renamed");
        assert!(!merge(&stored, &incoming, true).shared);
    }

    #[test]
    fn ties_are_settled_the_same_way_in_either_order() {
        let a = note(vec![block("a", "one", 4)]);
        let b = note(vec![block("a", "two", 4)]);
        assert_eq!(merge(&a, &b, true).blocks, merge(&b, &a, true).blocks);
    }

    #[test]
    fn preview_skips_deleted_and_orders_by_position() {
        let mut first = block("z", "first", 1);
        first.pos = 0.5;
        let mut gone = block("y", "gone", 1);
        gone.deleted = true;
        let n = note(vec![block("a", "second", 1), gone, first]);
        assert_eq!(n.preview(5), vec!["☐ first", "☐ second"]);
    }

    #[test]
    fn validate_rejects_bad_links() {
        let mut b = block("a", "", 1);
        b.kind = BlockKind::Link;
        b.url = "javascript:alert(1)".into();
        assert!(validate(&note(vec![b])).is_err());
    }
}
