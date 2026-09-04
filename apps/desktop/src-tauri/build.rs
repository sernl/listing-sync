/// Where a build is handed the fleet's entitlement public keys.
///
/// A variable rather than a secret, and public keys rather than private ones:
/// nothing here signs anything. The release workflow sets it from a repository
/// variable; `just desktop-dev` sets it from the development key pair; a clone
/// that sets nothing gets no keys at all.
///
/// One key, or two separated by a comma. Two exists for rotation: a release
/// carrying the outgoing and the incoming key verifies tokens signed by either,
/// so the fleet can be moved across in the order that strands nobody — ship the
/// dual-key client, wait for it to reach the fleet, switch the server, drop the
/// outgoing key in the next release.
const PUBLIC_KEY_ENV: &str = "TAM_ENTITLEMENT_PUBLIC_KEY";

/// Ed25519 public keys are thirty-two bytes, carried here as lowercase hex.
const PUBLIC_KEY_BYTES: usize = 32;

/// At most two, because the only reason for a second is a rotation in flight
/// and a third would mean two rotations overlapping — which is a situation to
/// resolve rather than to support.
const PUBLIC_KEYS_MAX: usize = 2;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `tauri::generate_context!` panics outright when `frontendDist` does not
    // exist, and it exists only once `custom-protocol` is on -- which is what
    // `just check`'s `--all-features` does. The console's build output is a
    // gitignored npm artefact, so on a clone that has not run `just web-check`
    // the gated lane would fail in a proc macro. An empty directory embeds
    // nothing and satisfies the check; `just desktop-build` fills it before it
    // bundles anything.
    std::fs::create_dir_all("../../../web/build").ok();
    emit_entitlement_keys()?;
    tauri_build::build();
    Ok(())
}

/// Writes the slice literal `entitlement::EMBEDDED_PUBLIC_KEYS` is defined as.
///
/// The decode lives here rather than in a const function beside the constant so
/// that a malformed key fails the build with a sentence, which a const
/// evaluation cannot do without a panic the lint table denies.
///
/// An unset variable emits an *empty* slice, and that is the whole point rather
/// than a convenience. The previous design emitted thirty-two zero bytes and
/// called them a key that verifies nothing; they are in fact a valid Ed25519
/// point of order four, against which a signature can be forged with no private
/// key at all, so "this build has no key" has to be a state the type carries
/// rather than a value that was assumed to fail. A clone, `just check` and every
/// test build have no key and must still compile; what they get is a set nothing
/// can verify against, because there is nothing in it.
fn emit_entitlement_keys() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo::rerun-if-env-changed={PUBLIC_KEY_ENV}");
    let keys = match supplied(PUBLIC_KEY_ENV) {
        Some(raw) => decode_keys(&raw)?,
        None => Vec::new(),
    };
    let literal: Vec<String> = keys
        .iter()
        .map(|key| {
            let bytes: Vec<String> = key.iter().map(|byte| format!("0x{byte:02x}")).collect();
            format!("[{}]", bytes.join(", "))
        })
        .collect();
    let out = std::path::Path::new(&supplied("OUT_DIR").ok_or("cargo set no OUT_DIR")?)
        .join("entitlement_key.rs");
    std::fs::write(out, format!("&[{}]", literal.join(", ")))?;
    Ok(())
}

/// One environment read, at the one boundary a build script is.
#[expect(
    clippy::disallowed_methods,
    reason = "a build script is a configuration-reading process boundary; this is the one site \
              that reads one, which is what the ban asks for"
)]
fn supplied(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|raw| raw.trim().to_owned())
        .filter(|raw| !raw.is_empty())
}

/// The comma-separated list to the keys it spells.
///
/// Every refusal here is a build failure with a sentence, because each one is a
/// misconfiguration that would otherwise ship a client that verifies the wrong
/// thing or nothing.
fn decode_keys(raw: &str) -> Result<Vec<[u8; PUBLIC_KEY_BYTES]>, String> {
    let mut keys = Vec::new();
    for field in raw.split(',') {
        let key = decode_key(field.trim())?;
        if keys.contains(&key) {
            return Err(format!(
                "{PUBLIC_KEY_ENV} names the same key twice; two keys exist for a rotation, \
                 and a duplicate means one of them was meant to be the other"
            ));
        }
        keys.push(key);
    }
    if keys.len() > PUBLIC_KEYS_MAX {
        return Err(format!(
            "{PUBLIC_KEY_ENV} names {} keys; at most {PUBLIC_KEYS_MAX} are accepted, the second \
             being the incoming key of a rotation in flight",
            keys.len()
        ));
    }
    Ok(keys)
}

/// Sixty-four lowercase hex characters to the thirty-two bytes they spell.
///
/// Lowercase and exactly that length, both refused rather than normalised: a key
/// of the wrong length is not a key, and accepting two spellings of one key would
/// let the variable and the record of what a release shipped differ while looking
/// identical.
///
/// All-zero is refused by name. It is a valid point of order four rather than the
/// inert value the previous design took it for, so a build that embedded it would
/// accept forged tokens; it is also exactly what a shell that expanded an unset
/// variable into zeroes would produce, which is the accident worth naming.
fn decode_key(hex: &str) -> Result<[u8; PUBLIC_KEY_BYTES], String> {
    let digits: Vec<u8> = hex.bytes().collect();
    if digits.len() != PUBLIC_KEY_BYTES * 2 {
        return Err(format!(
            "{PUBLIC_KEY_ENV} must be exactly {} lowercase hex characters per key; one has {}",
            PUBLIC_KEY_BYTES * 2,
            digits.len()
        ));
    }
    let mut bytes = [0u8; PUBLIC_KEY_BYTES];
    for (slot, pair) in bytes.iter_mut().zip(digits.chunks_exact(2)) {
        let mut value = 0u8;
        for digit in pair {
            let nibble = match digit {
                b'0'..=b'9' => digit - b'0',
                b'a'..=b'f' => digit - b'a' + 10,
                _ => {
                    return Err(format!(
                        "{PUBLIC_KEY_ENV} is not lowercase hex: {:?} is not a hex digit",
                        char::from(*digit)
                    ))
                }
            };
            value = value * 16 + nibble;
        }
        *slot = value;
    }
    if bytes == [0u8; PUBLIC_KEY_BYTES] {
        return Err(format!(
            "{PUBLIC_KEY_ENV} is all zeroes, which is a small-order Ed25519 point a signature \
             can be forged against without any private key; leave the variable unset to build \
             a client that verifies nothing"
        ));
    }
    Ok(bytes)
}
