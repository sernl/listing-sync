//! The operator session mint: find-or-create the user for an email under an
//! organisation, mint a session, and print the cookie token exactly once.
//! Self-serve signup is M5's; until then this one-shot is the login flow.
//! Entropy and the clock enter the repository as data from this boundary.
//!
//! Usage: tam-mint-session <db-url> <org-uuid-hex> <email> [ttl-days] [--ensure-org <name>]

#![forbid(unsafe_code)]

use tam_storage::{SessionRepo, SessionToken};
use tam_types::{OrgId, Timestamp, UserId, Uuid};

const DEFAULT_TTL_DAYS: i64 = 30;
const MILLIS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

fn org_from_hex(hex: &str) -> Result<OrgId, Box<dyn std::error::Error>> {
    if hex.len() != 32 {
        return Err("org hex must be 32 characters".into());
    }
    let mut array = [0u8; 16];
    for (index, slot) in array.iter_mut().enumerate() {
        let start = index * 2;
        let pair = hex.get(start..start + 2).ok_or("org hex is malformed")?;
        *slot = u8::from_str_radix(pair, 16)?;
    }
    Ok(OrgId(Uuid(array)))
}

#[expect(
    clippy::disallowed_methods,
    reason = "the mint is a clock-reading process boundary; time enters the session rows as data from here"
)]
fn wall_now() -> Result<Timestamp, Box<dyn std::error::Error>> {
    Ok(Timestamp(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?))
}

/// Two v4 UUIDs concatenated: 32 bytes of operating-system randomness via the
/// same generator the workspace already trusts for row identity.
fn fresh_token() -> SessionToken {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    SessionToken(bytes)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = Vec::new();
    let mut ensure_org_name: Option<String> = None;
    let mut raw = std::env::args().skip(1);
    while let Some(argument) = raw.next() {
        if argument == "--ensure-org" {
            ensure_org_name = Some(raw.next().ok_or("--ensure-org needs a name argument")?);
        } else {
            arguments.push(argument);
        }
    }
    let db_url = arguments.first().ok_or("missing db url")?;
    let org = org_from_hex(arguments.get(1).ok_or("missing org hex")?)?;
    let email = arguments.get(2).ok_or("missing email")?;
    let ttl_days: i64 = match arguments.get(3) {
        Some(raw) => raw.parse()?,
        None => DEFAULT_TTL_DAYS,
    };
    if ttl_days <= 0 {
        return Err("ttl-days must be positive".into());
    }

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(db_url)
        .await?;
    let repo = SessionRepo::new(pool);
    let now = wall_now()?;

    if let Some(name) = &ensure_org_name {
        repo.ensure_org(org, name, now).await?;
    }

    let user = if let Some(existing) = repo.user_by_email(email).await? {
        existing
    } else {
        let minted = UserId(Uuid(*uuid::Uuid::new_v4().as_bytes()));
        repo.create_user(org, minted, email, now).await?;
        minted
    };

    let token = fresh_token();
    let expires_at = Timestamp(
        now.0
            .checked_add(
                ttl_days
                    .checked_mul(MILLIS_PER_DAY)
                    .ok_or("ttl overflows")?,
            )
            .ok_or("expiry overflows")?,
    );
    repo.mint(&token, user, expires_at, now).await?;

    eprintln!("session minted for {email}; expires in {ttl_days} days");
    // The product goes to stdout so a pipe captures exactly the paste line;
    // the status above stays on stderr.
    println!("tam_session={}", token.to_hex());
    Ok(())
}
