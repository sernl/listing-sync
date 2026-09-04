//! The entitlement key mint: the Ed25519 key pair decision D10's token is
//! signed and verified with.
//!
//! Run once by the founder, never by CI and never by a server. The private
//! half is written where `tam-server --entitlement-key-path` will read it; the
//! public half is printed, and becomes the `TAM_ENTITLEMENT_PUBLIC_KEY`
//! repository variable a desktop release embeds.
//!
//! The two halves are asymmetric in form as well as in secrecy, and both forms
//! are what their consumer takes rather than a choice made here. The private
//! half is PKCS#8 DER because `jsonwebtoken` is built without its `use_pem`
//! feature, so `from_ed_pem` does not exist in this workspace at all. The
//! public half is the raw thirty-two bytes as lowercase hex, because that is
//! what `DecodingKey::from_ed_der` verifies against and what the desktop build
//! script decodes.
//!
//! Unlike the updater's signing key this one is replaceable: losing it costs a
//! rotation, not the installed base. The rotation is lossless only because a
//! build can carry two public keys: set `TAM_ENTITLEMENT_PUBLIC_KEY` to
//! `<outgoing>,<incoming>` and release, wait for that build to reach the fleet,
//! switch this server to the incoming key, then drop the outgoing one in the
//! next release. Every step is safe in both directions, because throughout it
//! every installed client accepts tokens signed by whichever key the server is
//! currently using.
//!
//! With a single embedded key there is no safe order, which is worth stating
//! because this file recommended one until 2026-09-04. Releasing the new-key
//! client first strands whoever updates fastest -- their client cannot verify
//! the old-key token it receives an hour later and closes its gate immediately,
//! for the whole length of the wait -- and switching the server first strands
//! the slow updaters instead.
//!
//! Usage: tam-entitlement-key <output-path>

#![forbid(unsafe_code)]

use std::io::Write as _;

/// An Ed25519 public key is thirty-two bytes, and the build script that
/// embeds it wants exactly this many lowercase hex characters.
const PUBLIC_KEY_HEX_CHARS: usize = 64;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: tam-entitlement-key <output-path>")?;

    let random = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&random)
        .map_err(|_unspecified| "the system random source refused to generate a key")?;
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
        .map_err(|why| format!("the generated key does not parse: {why}"))?;

    write_private(&path, pkcs8.as_ref())?;

    let public = ring::signature::KeyPair::public_key(&pair);
    let mut hex = String::with_capacity(PUBLIC_KEY_HEX_CHARS);
    for byte in public.as_ref() {
        use core::fmt::Write as _;
        // infallible on String; the Result is the trait's, not the writer's
        let _unused: core::fmt::Result = write!(hex, "{byte:02x}");
    }
    println!("private key written to {path}");
    println!("public key (hex): {hex}");
    Ok(())
}

/// Writes the private half, refusing to overwrite.
///
/// `create_new` rather than `create`: a second run against a path that already
/// holds a production key would replace it silently, and the fleet verifying
/// against the old public half would then verify nothing until it updated.
/// Refusing costs one `rm` when the overwrite was actually intended.
#[cfg(unix)]
fn write_private(path: &str, der: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(der)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &str, der: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(der)?;
    println!("restrict {path} by hand: this platform has no mode to set it with");
    Ok(())
}
