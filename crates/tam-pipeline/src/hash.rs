//! Content addressing by blake3. The digest is the dedup key and, per tenant,
//! the blob primary key; dedup is per tenant, never global, because a global
//! hash table is an existence oracle over what every seller uploaded.

use tam_types::ContentHash;

#[must_use]
pub fn content_hash(bytes: &[u8]) -> ContentHash {
    ContentHash(blake3::hash(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::content_hash;

    #[test]
    fn the_hash_is_deterministic_and_content_sensitive() {
        assert_eq!(
            content_hash(b"payload"),
            content_hash(b"payload"),
            "identical bytes hash identically, which is what makes dedup work"
        );
        assert_ne!(
            content_hash(b"payload"),
            content_hash(b"payloaful"),
            "different bytes hash differently"
        );
    }
}
