//! A 128-bit digest of an object or feature id, for the sets that guard
//! uniqueness and contiguity over a whole dataset without holding every id
//! string in memory.

use std::hash::{DefaultHasher, Hash, Hasher};

/// Two independent SipHash digests of `id`; at 128 bits a collision among
/// any realistic number of ids is not a practical concern.
pub(crate) fn id_hash(id: &str) -> u128 {
    let mut high = DefaultHasher::new();
    id.hash(&mut high);
    let mut low = DefaultHasher::new();
    (0x5eed_u32, id).hash(&mut low);
    (u128::from(high.finish()) << 64) | u128::from(low.finish())
}
