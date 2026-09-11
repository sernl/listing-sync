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
    require_fallback_page()?;
    emit_entitlement_keys()?;
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )?;
    Ok(())
}

/// The application's own commands, declared so a capability can grant them.
///
/// Without this list Tauri's access control has no permissions to resolve for
/// an application command, so every invocation is refused before dispatch —
/// and refused identically whether the command is registered or not, which is
/// measurable: a mock application answers `start_import` and a name nothing
/// registers with the same `not allowed. Plugin not found`. That is also why
/// the console, once it is served from the control plane's origin rather than
/// the bundled bundle, needs this: remote content is granted nothing by
/// default, and this is the half that makes a grant expressible at all.
///
/// `retry_console` is granted LOCALLY rather than remotely: it is the fallback
/// page's one button, and that page is bundled, so it runs at the Tauri origin
/// where the console never does.
///
/// The list is exactly the handler list in `lib.rs`. A command in one and not
/// the other is either a command nothing can call or a permission for nothing,
/// and both are silent.
const COMMANDS: &[&str] = &[
    "connect_marketplace",
    "session_status",
    "forget_session",
    "device_check_in",
    "device_activity",
    "start_import",
    "retry_console",
    "set_theme",
];

/// A bundle that carries a console must carry the page shown when the console
/// cannot be reached.
///
/// Checked here because nothing else does. `generate_context!` embeds whatever
/// is in `web/build` and a missing file is not an error, so before this a
/// binary could ship with a console and no fallback — and the seller who most
/// needed the fallback, the one with no network, would get a blank window
/// instead. `web/static/unreachable.html` is copied into the build by
/// SvelteKit, so a build that ran `just web-check` has it and one that used a
/// stale directory does not.
///
/// Only when `index.html` is present, because a clone that has never built the
/// console must still compile: the empty directory above satisfies
/// `generate_context!` and there is no console to be missing a fallback from.
/// The release workflow builds the console before it bundles, so the shipping
/// path always takes the checked branch.
fn require_fallback_page() -> Result<(), Box<dyn std::error::Error>> {
    let build = std::path::Path::new("../../../web/build");
    println!("cargo::rerun-if-changed=../../../web/build/index.html");
    println!("cargo::rerun-if-changed=../../../web/build/unreachable.html");
    if build.join("index.html").exists() && !build.join("unreachable.html").exists() {
        return Err(
            "web/build has a console but no unreachable.html, so a bundle from it \
                    would show a blank window to any seller who cannot reach the server. \
                    Rebuild the console with `just web-check`, which copies web/static into \
                    web/build."
                .into(),
        );
    }
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
