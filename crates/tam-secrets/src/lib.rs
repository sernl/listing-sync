//! The credential envelope: a seller's marketplace secret sealed so that only
//! the process holding the key-encryption key can open it, and only in the
//! exact tenant context it was sealed in.
//!
//! Two layers of XChaCha20-Poly1305, per the design's custody section. A fresh
//! 32-byte data-encryption key seals the secret; the key-encryption key seals
//! that DEK. Both layers bind the same additional authenticated data —
//! `org || marketplace || connection || key_version` — so a `connection_secret`
//! row lifted into another tenant's context fails authentication rather than
//! decrypting into the wrong seller. An attacker with a database dump but not
//! the KEK holds ciphertext; that is the whole point of the process boundary
//! the broker enforces around this crate.

#![forbid(unsafe_code)]

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand_core::{OsRng, RngCore};
use tam_types::OrgId;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A secret held so it cannot leak through a log line or a stray clone: its
/// `Debug` redacts, it zeroizes on drop, and it is moved by value into the
/// seal so no copy outlives the sealing.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct Secret(String);

impl Secret {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// A borrow for the one legitimate use — sealing, or handing to the
    /// gateway that injects it. Never logged.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for Secret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Secret(redacted)")
    }
}

/// The key-encryption key: 32 bytes, zeroizing, read from a file the operating
/// system delivers (systemd `LoadCredentialEncrypted=` in production, a mode-
/// 0600 file in development — the same code path either way, which is what
/// makes the escrow-recovery test meaningful).
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Kek([u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KekError {
    WrongLength { found: usize },
}

impl core::fmt::Display for KekError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WrongLength { found } => {
                write!(f, "a key-encryption key is 32 bytes, found {found}")
            }
        }
    }
}

impl core::error::Error for KekError {}

impl Kek {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, KekError> {
        <[u8; 32]>::try_from(bytes)
            .map(Self)
            .map_err(|_| KekError::WrongLength { found: bytes.len() })
    }
}

/// The tenant context bound as additional authenticated data at both layers.
/// Fixed-width canonical encoding, so no separator is needed and a row cannot
/// be reinterpreted under a different context by field-boundary confusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AadContext {
    pub org: OrgId,
    pub marketplace: Marketplace,
    pub connection: ConnectionId,
    pub key_version: i32,
}

pub use tam_types::{ConnectionId, Marketplace};

impl AadContext {
    fn encode(&self) -> [u8; 37] {
        let mut aad = [0u8; 37];
        aad[0..16].copy_from_slice(&self.org.0 .0);
        aad[16] = marketplace_ordinal(self.marketplace);
        aad[17..33].copy_from_slice(&self.connection.0 .0);
        aad[33..37].copy_from_slice(&self.key_version.to_be_bytes());
        aad
    }
}

/// Part of the AAD encoding and therefore forever: append variants, never
/// renumber, or every stored row's context silently changes.
const fn marketplace_ordinal(marketplace: Marketplace) -> u8 {
    match marketplace {
        Marketplace::Tes => 0,
        Marketplace::Etsy => 1,
        Marketplace::Tpt => 2,
    }
}

/// A sealed credential: exactly the columns `connection_secret` stores. The
/// wrapped DEK carries its own nonce as its first 24 bytes; the ciphertext
/// carries its nonce in `nonce`.
#[derive(Clone, PartialEq, Eq, Zeroize)]
pub struct Sealed {
    pub key_version: i32,
    pub wrapped_dek: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

impl core::fmt::Debug for Sealed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Sealed")
            .field("key_version", &self.key_version)
            .field("wrapped_dek", &"<ciphertext>")
            .field("nonce", &"<nonce>")
            .field("ciphertext", &"<ciphertext>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealError {
    Crypto,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// The key or the context did not match, or the material was tampered
    /// with; the three are deliberately indistinguishable, because
    /// distinguishing them is an oracle.
    Authentication,
    Truncated,
    NotUtf8,
}

impl core::fmt::Display for OpenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Authentication => f.write_str("the sealed credential did not authenticate"),
            Self::Truncated => f.write_str("the sealed material is truncated"),
            Self::NotUtf8 => f.write_str("the opened plaintext is not valid UTF-8"),
        }
    }
}

impl core::error::Error for OpenError {}

/// A blob's encryption context: the tenant and the content hash bind the
/// object bytes, so a blob lifted into another tenant fails authentication.
/// Separate from `AadContext` because a blob has no marketplace or connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobAad {
    pub org: OrgId,
    pub hash: [u8; 32],
    pub key_version: i32,
}

impl BlobAad {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut aad = Vec::with_capacity(52);
        aad.extend_from_slice(&self.org.0 .0);
        aad.extend_from_slice(&self.hash);
        aad.extend_from_slice(&self.key_version.to_be_bytes());
        aad
    }
}

const XNONCE_LEN: usize = 24;

fn random_nonce() -> XNonce {
    let mut bytes = [0u8; XNONCE_LEN];
    OsRng.fill_bytes(&mut bytes);
    XNonce::from(bytes)
}

/// Seals a secret under a fresh DEK, then seals the DEK under the KEK, both
/// bound to `context`. The secret is consumed by value so no copy of it
/// outlives the seal — that by-value move is the point, not an oversight.
#[expect(
    clippy::needless_pass_by_value,
    reason = "consuming the secret by value is the security property: no copy of the plaintext outlives the seal"
)]
pub fn seal(kek: &Kek, context: &AadContext, secret: Secret) -> Result<Sealed, SealError> {
    let aad = context.encode();
    let mut dek = [0u8; 32];
    OsRng.fill_bytes(&mut dek);

    let payload_cipher = XChaCha20Poly1305::new((&dek).into());
    let payload_nonce = random_nonce();
    let ciphertext = payload_cipher
        .encrypt(
            &payload_nonce,
            Payload {
                msg: secret.expose().as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|_| SealError::Crypto)?;

    let wrap_cipher = XChaCha20Poly1305::new((&kek.0).into());
    let wrap_nonce = random_nonce();
    let wrapped = wrap_cipher
        .encrypt(
            &wrap_nonce,
            Payload {
                msg: &dek,
                aad: &aad,
            },
        )
        .map_err(|_| SealError::Crypto)?;
    dek.zeroize();

    // The wrap nonce rides at the front of wrapped_dek; the payload nonce is
    // its own column.
    let mut wrapped_dek = Vec::with_capacity(XNONCE_LEN + wrapped.len());
    wrapped_dek.extend_from_slice(wrap_nonce.as_slice());
    wrapped_dek.extend_from_slice(&wrapped);

    Ok(Sealed {
        key_version: context.key_version,
        wrapped_dek,
        nonce: payload_nonce.as_slice().to_vec(),
        ciphertext,
    })
}

/// Opens a sealed credential, reversing `seal`. Every failure is the same
/// opaque `Authentication` where a wrong key, a wrong context or a tampered
/// byte could be told apart, because telling them apart is an oracle.
pub fn open(kek: &Kek, context: &AadContext, sealed: &Sealed) -> Result<Secret, OpenError> {
    let aad = context.encode();
    if sealed.wrapped_dek.len() <= XNONCE_LEN || sealed.nonce.len() != XNONCE_LEN {
        return Err(OpenError::Truncated);
    }
    let (wrap_nonce, wrapped) = sealed.wrapped_dek.split_at(XNONCE_LEN);

    let wrap_cipher = XChaCha20Poly1305::new((&kek.0).into());
    let mut dek = wrap_cipher
        .decrypt(
            XNonce::from_slice(wrap_nonce),
            Payload {
                msg: wrapped,
                aad: &aad,
            },
        )
        .map_err(|_| OpenError::Authentication)?;

    let payload_cipher = XChaCha20Poly1305::new(dek.as_slice().into());
    let plaintext = payload_cipher
        .decrypt(
            XNonce::from_slice(&sealed.nonce),
            Payload {
                msg: &sealed.ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| OpenError::Authentication);
    dek.zeroize();
    let plaintext = plaintext?;

    String::from_utf8(plaintext)
        .map(Secret::new)
        .map_err(|_| OpenError::NotUtf8)
}

/// Seals raw bytes under a caller-supplied AAD, for blob objects. Same
/// two-layer envelope as `seal`; the payload is binary rather than a UTF-8
/// secret.
pub fn seal_bytes(kek: &Kek, aad: &[u8], plaintext: &[u8]) -> Result<Sealed, SealError> {
    let mut dek = [0u8; 32];
    OsRng.fill_bytes(&mut dek);

    let payload_cipher = XChaCha20Poly1305::new((&dek).into());
    let payload_nonce = random_nonce();
    let ciphertext = payload_cipher
        .encrypt(
            &payload_nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| SealError::Crypto)?;

    let wrap_cipher = XChaCha20Poly1305::new((&kek.0).into());
    let wrap_nonce = random_nonce();
    let wrapped = wrap_cipher
        .encrypt(&wrap_nonce, Payload { msg: &dek, aad })
        .map_err(|_| SealError::Crypto)?;
    dek.zeroize();

    let mut wrapped_dek = Vec::with_capacity(XNONCE_LEN + wrapped.len());
    wrapped_dek.extend_from_slice(wrap_nonce.as_slice());
    wrapped_dek.extend_from_slice(&wrapped);

    Ok(Sealed {
        key_version: 1,
        wrapped_dek,
        nonce: payload_nonce.as_slice().to_vec(),
        ciphertext,
    })
}

/// Opens bytes sealed by `seal_bytes` under the same AAD.
pub fn open_bytes(kek: &Kek, aad: &[u8], sealed: &Sealed) -> Result<Vec<u8>, OpenError> {
    if sealed.wrapped_dek.len() <= XNONCE_LEN || sealed.nonce.len() != XNONCE_LEN {
        return Err(OpenError::Truncated);
    }
    let (wrap_nonce, wrapped) = sealed.wrapped_dek.split_at(XNONCE_LEN);

    let wrap_cipher = XChaCha20Poly1305::new((&kek.0).into());
    let mut dek = wrap_cipher
        .decrypt(
            XNonce::from_slice(wrap_nonce),
            Payload { msg: wrapped, aad },
        )
        .map_err(|_| OpenError::Authentication)?;

    let payload_cipher = XChaCha20Poly1305::new(dek.as_slice().into());
    let plaintext = payload_cipher
        .decrypt(
            XNonce::from_slice(&sealed.nonce),
            Payload {
                msg: &sealed.ciphertext,
                aad,
            },
        )
        .map_err(|_| OpenError::Authentication);
    dek.zeroize();
    plaintext
}

/// Domain separation for the account-digest pepper: this label and no other
/// use of the key-encryption key produce this subkey. The pepper is a one-way
/// function of the KEK, so a leaked pepper does not yield the KEK and cannot
/// open a single sealed credential.
const ACCOUNT_DIGEST_LABEL: &[u8] = b"tam:platform-account-digest:v1";

/// Separates the marketplace ordinal from the account reference so that
/// `(marketplace, account_ref)` has exactly one encoding. Without it, two
/// distinct pairs could concatenate to the same bytes and collide into one
/// exclusivity lock.
const FIELD_SEPARATOR: u8 = 0x1f;

/// The SHA-256 block size, which is what RFC 2104 pads the key to.
const HMAC_BLOCK: usize = 64;

/// HMAC-SHA-256, RFC 2104, over the `sha2` this workspace already pins.
///
/// Written out rather than pulled from a MAC crate because the construction
/// is short, fully specified, and pinned here by the RFC 4231 vectors; adding
/// a dependency to the one crate that holds key material is a larger decision
/// than the twenty lines it saves.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};

    let mut padded = [0u8; HMAC_BLOCK];
    if key.len() > HMAC_BLOCK {
        padded[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        padded[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36u8; HMAC_BLOCK];
    let mut outer_pad = [0x5cu8; HMAC_BLOCK];
    for ((inner, outer), key_byte) in inner_pad
        .iter_mut()
        .zip(outer_pad.iter_mut())
        .zip(padded.iter())
    {
        *inner ^= *key_byte;
        *outer ^= *key_byte;
    }
    let inner = Sha256::new()
        .chain_update(inner_pad)
        .chain_update(message)
        .finalize();
    let outer = Sha256::new()
        .chain_update(outer_pad)
        .chain_update(inner)
        .finalize();
    padded.zeroize();
    inner_pad.zeroize();
    outer_pad.zeroize();
    outer.into()
}

/// A stable, non-reversible name for the marketplace account a connection
/// speaks for.
///
/// Global exclusivity is enforced on this value rather than on the account
/// reference itself, so a database dump names no seller's storefront and no
/// two tenants' rows can be correlated by eye. Recovering an identifier from a
/// digest, or confirming a guessed one, needs the key-encryption key, which
/// lives only in the broker process — the same boundary the credential vault
/// already sits behind.
///
/// `key_version` is bound into the digest so that a rotation is loud rather
/// than silent. Rotating the KEK rotates the pepper, so every stored digest
/// stops matching anything freshly computed — and because exclusivity is
/// enforced on equality, a silent rotation would not merely fail to match, it
/// would let a second organisation claim an account the first still holds
/// under the retired key. With the version in the input the two generations
/// are visibly different values, and re-claiming every connection is a
/// recognised step of a rotation rather than an unnoticed loss of the lock.
#[must_use]
pub fn account_digest(
    kek: &Kek,
    marketplace: Marketplace,
    key_version: i32,
    account_ref: &str,
) -> [u8; 32] {
    let mut pepper = hmac_sha256(&kek.0, ACCOUNT_DIGEST_LABEL);
    let mut message = Vec::with_capacity(account_ref.len() + 7);
    message.push(marketplace_ordinal(marketplace));
    message.push(FIELD_SEPARATOR);
    message.extend_from_slice(&key_version.to_be_bytes());
    message.push(FIELD_SEPARATOR);
    message.extend_from_slice(account_ref.as_bytes());
    let digest = hmac_sha256(&pepper, &message);
    pepper.zeroize();
    digest
}

/// One nibble as a lowercase hex character.
///
/// Total by construction: every caller masks to four bits, so no value above
/// fifteen reaches it and there is no error case to represent.
fn hex_digit(nibble: u8) -> char {
    if nibble < 10 {
        char::from(b'0' + nibble)
    } else {
        char::from(b'a' + nibble - 10)
    }
}

/// Lowercase hex, written a nibble at a time rather than a `format!` per byte.
#[must_use]
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(hex_digit(byte >> 4));
        encoded.push(hex_digit(byte & 0x0f));
    }
    encoded
}

/// A fresh random token, hex-encoded, for the per-lease bearer credential.
///
/// Sourced from the same `OsRng` the nonces and data-encryption keys come
/// from, so the lease token is no weaker than the envelope it guards.
#[must_use]
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token = hex_encode(&bytes);
    bytes.zeroize();
    token
}

#[cfg(test)]
mod tests {
    use super::{
        account_digest, hmac_sha256, open, random_token, seal, AadContext, Kek, OpenError, Secret,
    };
    use tam_types::{ConnectionId, Marketplace, OrgId, Uuid};

    fn kek(seed: u8) -> Kek {
        Kek::from_bytes(&[seed; 32]).expect("32 bytes is a key")
    }

    fn context(org: u8) -> AadContext {
        AadContext {
            org: OrgId(Uuid([org; 16])),
            marketplace: Marketplace::Tes,
            connection: ConnectionId(Uuid([0x33; 16])),
            key_version: 1,
        }
    }

    #[test]
    fn a_sealed_secret_round_trips() {
        let sealed = seal(
            &kek(0x01),
            &context(0xAA),
            Secret::new("s3ssion=abc".to_owned()),
        )
        .expect("sealing succeeds");
        let opened = open(&kek(0x01), &context(0xAA), &sealed).expect("opening succeeds");
        assert_eq!(
            opened.expose(),
            "s3ssion=abc",
            "the credential survives the envelope unchanged"
        );
    }

    #[test]
    fn escrow_recovery_opens_with_a_reloaded_key() {
        // The KEK is escrowed and later reloaded from its escrow copy — the
        // motherboard-failure recovery the design requires to exist before any
        // customer data does. Modelled as reconstructing the key from the same
        // bytes a real escrow would hold.
        let original = kek(0x02);
        let sealed = seal(
            &original,
            &context(0xAA),
            Secret::new("recover-me".to_owned()),
        )
        .expect("sealing succeeds");
        drop(original);
        let escrow_copy = Kek::from_bytes(&[0x02; 32]).expect("the escrowed bytes rebuild the key");
        let opened =
            open(&escrow_copy, &context(0xAA), &sealed).expect("the reloaded key opens the seal");
        assert_eq!(
            opened.expose(),
            "recover-me",
            "a credential sealed before a hardware loss is recoverable from key escrow"
        );
    }

    #[test]
    fn the_wrong_key_cannot_open() {
        let sealed =
            seal(&kek(0x03), &context(0xAA), Secret::new("secret".to_owned())).expect("seals");
        assert_eq!(
            open(&kek(0x04), &context(0xAA), &sealed).err(),
            Some(OpenError::Authentication),
            "a dump without the key yields ciphertext, not the secret"
        );
    }

    #[test]
    fn a_row_replayed_into_another_tenant_fails_authentication() {
        let sealed = seal(
            &kek(0x05),
            &context(0xAA),
            Secret::new("tenant-a".to_owned()),
        )
        .expect("seals under tenant A");
        assert_eq!(
            open(&kek(0x05), &context(0xBB), &sealed).err(),
            Some(OpenError::Authentication),
            "the same ciphertext under another org's context must not decrypt"
        );
    }

    #[test]
    fn a_truncated_seal_is_refused_before_any_crypto() {
        let mut sealed =
            seal(&kek(0x06), &context(0xAA), Secret::new("x".to_owned())).expect("seals");
        sealed.nonce.truncate(4);
        assert_eq!(
            open(&kek(0x06), &context(0xAA), &sealed).err(),
            Some(OpenError::Truncated),
            "malformed material fails closed"
        );
    }

    #[test]
    fn blob_bytes_round_trip_and_resist_cross_tenant_replay() {
        use super::{open_bytes, seal_bytes, BlobAad};
        let kek = kek(0x07);
        let aad_a = BlobAad {
            org: OrgId(Uuid([0xAA; 16])),
            hash: [0x11; 32],
            key_version: 1,
        };
        let sealed = seal_bytes(&kek, &aad_a.encode(), b"a resource file's bytes").expect("seal");
        let opened = open_bytes(&kek, &aad_a.encode(), &sealed).expect("open");
        assert_eq!(opened, b"a resource file's bytes", "blob bytes round-trip");

        let aad_b = BlobAad {
            org: OrgId(Uuid([0xBB; 16])),
            hash: [0x11; 32],
            key_version: 1,
        };
        assert_eq!(
            open_bytes(&kek, &aad_b.encode(), &sealed).err(),
            Some(super::OpenError::Authentication),
            "a blob object lifted into another tenant fails authentication"
        );
    }

    /// RFC 4231's own vectors, so the MAC is pinned to the standard rather
    /// than to itself. Without these, a transposed constant would produce a
    /// self-consistent function that is not HMAC and every digest built on it
    /// would be silently non-standard.
    use super::hex_encode as hex;

    #[test]
    fn hmac_matches_rfc_4231_case_1() {
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
            "HMAC-SHA-256 must reproduce RFC 4231 test case 1"
        );
    }

    #[test]
    fn hmac_matches_rfc_4231_case_2() {
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            "HMAC-SHA-256 must reproduce RFC 4231 test case 2"
        );
    }

    #[test]
    fn hmac_matches_rfc_4231_case_6_where_the_key_exceeds_the_block() {
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
            "a key longer than the block must be hashed first, per RFC 2104"
        );
    }

    #[test]
    fn a_digest_is_stable_for_the_same_account_under_the_same_key() {
        let first = account_digest(&kek(0x07), Marketplace::Tpt, 1, "900000001");
        let second = account_digest(&kek(0x07), Marketplace::Tpt, 1, "900000001");
        assert_eq!(
            first, second,
            "the exclusivity lock is the digest, so the same account must digest identically \
             or a relink would collide with itself"
        );
    }

    #[test]
    fn the_same_account_reference_on_two_marketplaces_digests_differently() {
        let tpt = account_digest(&kek(0x07), Marketplace::Tpt, 1, "12345");
        let tes = account_digest(&kek(0x07), Marketplace::Tes, 1, "12345");
        assert_ne!(
            tpt, tes,
            "the index is (marketplace, digest), but a shared digest would still let one \
             marketplace's account reference be recognised in another's rows"
        );
    }

    #[test]
    fn the_field_separator_stops_a_concatenation_collision() {
        let split_one = account_digest(&kek(0x07), Marketplace::Tpt, 1, "1\u{1f}23");
        let split_two = account_digest(&kek(0x07), Marketplace::Tpt, 1, "1\u{1f}2\u{1f}3");
        assert_ne!(
            split_one, split_two,
            "two distinct account references must not encode to the same message"
        );
    }

    #[test]
    fn rotating_the_key_rotates_the_digest() {
        assert_ne!(
            account_digest(&kek(0x07), Marketplace::Tpt, 1, "900000001"),
            account_digest(&kek(0x08), Marketplace::Tpt, 1, "900000001"),
            "the pepper derives from the KEK, so a rotation must invalidate stored digests \
             rather than silently keeping them valid under a retired key"
        );
    }

    #[test]
    fn a_key_version_bump_changes_the_digest() {
        assert_ne!(
            account_digest(&kek(0x07), Marketplace::Tpt, 1, "900000001"),
            account_digest(&kek(0x07), Marketplace::Tpt, 2, "900000001"),
            "exclusivity is enforced on digest equality, so two key generations must produce \
             visibly different values rather than a rotation silently releasing a held account"
        );
    }

    #[test]
    fn the_pepper_is_not_the_key_itself() {
        let key = kek(0x09);
        let digest = account_digest(&key, Marketplace::Tpt, 1, "900000001");
        assert_ne!(
            digest.as_slice(),
            &[0x09; 32],
            "a digest that exposed the key-encryption key would make every stored row's \
             pepper an oracle for the vault"
        );
    }

    #[test]
    fn two_lease_tokens_differ_and_are_full_length() {
        let first = random_token();
        let second = random_token();
        assert_eq!(
            first.len(),
            64,
            "32 random bytes hex-encode to 64 characters"
        );
        assert_ne!(
            first, second,
            "a lease token that repeated would let a retired lease's holder ride a new one"
        );
    }

    #[test]
    fn a_secret_redacts_in_debug() {
        let secret = Secret::new("TESSession=leak-me".to_owned());
        assert_eq!(
            format!("{secret:?}"),
            "Secret(redacted)",
            "a secret must never print its value"
        );
    }
}
