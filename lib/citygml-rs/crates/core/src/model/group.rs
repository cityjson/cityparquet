//! `CityObjectGroup` membership: a group's members, and the index that tells
//! a member which groups reference it.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

/// One member of a `CityObjectGroup` (`grp:groupMember`).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupMember {
    pub target: MemberTarget,
    /// Its `role` in the group, when the document gives one.
    pub role: Option<String>,
}

/// Where a group member is.
#[derive(Debug, Clone, PartialEq)]
pub enum MemberTarget {
    /// The group's child at this index of [`super::GmlObject::children`].
    Child(usize),
    /// An object elsewhere in the document, by `gml:id`.
    Id(String),
}

/// Member `gml:id` -> the groups that reference it, in document order.
///
/// A document can hold millions of members, so a member is keyed by a
/// 128-bit hash of its id rather than the id itself, and each group id is
/// stored once. A member of one group, the usual case, costs one map entry.
#[derive(Debug, Default)]
pub struct GroupIndex {
    groups: Vec<String>,
    group_ids: HashMap<String, u32>,
    first: HashMap<u128, u32>,
    /// The second and later groups of a member of several.
    more: HashMap<u128, Vec<u32>>,
}

impl GroupIndex {
    /// Record that group `group` references `member`. A repeated reference,
    /// or a group referencing itself, is recorded once or not at all.
    pub fn add(&mut self, group: &str, member: &str) {
        if group == member {
            return;
        }
        let g = match self.group_ids.get(group) {
            Some(&g) => g,
            None => {
                let g = self.groups.len() as u32;
                self.groups.push(group.to_string());
                self.group_ids.insert(group.to_string(), g);
                g
            }
        };
        let key = key(member);
        match self.first.get(&key) {
            None => {
                self.first.insert(key, g);
            }
            Some(&first) if first == g => {}
            Some(_) => {
                let more = self.more.entry(key).or_default();
                if !more.contains(&g) {
                    more.push(g);
                }
            }
        }
    }

    /// The groups that reference `member`, in document order.
    pub fn groups_of(&self, member: &str) -> impl Iterator<Item = &str> {
        let key = key(member);
        self.first
            .get(&key)
            .into_iter()
            .chain(self.more.get(&key).into_iter().flatten())
            .map(|&g| self.groups[g as usize].as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.first.is_empty()
    }
}

/// Two independent 64-bit SipHashes (fixed keys, so the same in every run).
fn key(id: &str) -> u128 {
    let mut a = DefaultHasher::new();
    id.hash(&mut a);
    let mut b = DefaultHasher::new();
    (0x5au8, id).hash(&mut b);
    (u128::from(a.finish()) << 64) | u128::from(b.finish())
}
