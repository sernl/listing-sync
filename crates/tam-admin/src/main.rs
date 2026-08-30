//! The operator command-line tool: grants the platform-operator marking,
//! withdraws it, and lists who holds it.
//!
//! This is the only way an operator comes into existence. No HTTP path
//! creates or modifies the marking, so there is no self-elevation endpoint to
//! attack, and the first operator is bootstrapped by someone who already has
//! the database credentials on the box rather than by a seeded migration that
//! cannot know whose uuid the founder's is.
//!
//! One pass per invocation, like `tam-mint-session`: it connects, does the one
//! thing, prints the result and exits. A non-zero status means the marking is
//! not in the state the command asked for -- including a revoke that found
//! nothing active to withdraw, because a command run to remove access must not
//! report success when it removed none.
//!
//! Usage: tam-admin <db-url> grant  (--user <uuid> | --email <address>) [--by <who>]
//!        tam-admin <db-url> revoke (--user <uuid> | --email <address>)
//!        tam-admin <db-url> list

#![forbid(unsafe_code)]

use tam_storage::{OperatorRepo, SessionRepo};
use tam_types::{Timestamp, UserId, Uuid};

const USAGE: &str =
    "usage: tam-admin <db-url> grant  (--user <uuid> | --email <address>) [--by <who>]\n\
                     \x20      tam-admin <db-url> revoke (--user <uuid> | --email <address>)\n\
                     \x20      tam-admin <db-url> list";

/// What `granted_by` carries when nobody named a granter. The bootstrap grant
/// is made from the box before any operator exists to attribute it to, and
/// recording the tool that made it is the one honest answer available: naming
/// the founder would be this command inventing evidence.
const DEFAULT_GRANTED_BY: &str = "tam-admin";

#[expect(
    clippy::disallowed_methods,
    reason = "the one-shot is a clock-reading process boundary; time enters the marking as data from here"
)]
fn wall_now() -> Result<Timestamp, Box<dyn std::error::Error>> {
    Ok(Timestamp(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?))
}

/// The usage text goes to stderr and the error itself stays one line.
///
/// A binary whose `main` returns `Err` renders that error through `Debug`, so
/// a multi-line usage message returned as the error prints with its newlines
/// escaped rather than as lines.
fn usage_error() -> Box<dyn std::error::Error> {
    eprintln!("{USAGE}");
    "missing arguments".into()
}

/// How the command names the human it acts on. Exactly one of the two, so a
/// call naming both is refused rather than silently preferring one.
enum Subject {
    User(UserId),
    Email(String),
}

enum Command {
    Grant {
        subject: Subject,
        granted_by: String,
    },
    Revoke {
        subject: Subject,
    },
    List,
}

struct Invocation {
    db_url: String,
    command: Command,
}

fn user_from_uuid(raw: &str) -> Result<UserId, Box<dyn std::error::Error>> {
    let parsed = uuid::Uuid::parse_str(raw)?;
    Ok(UserId(Uuid(*parsed.as_bytes())))
}

fn parse_invocation() -> Result<Invocation, Box<dyn std::error::Error>> {
    let mut positional = Vec::new();
    let mut user = None;
    let mut email = None;
    let mut granted_by = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--user" => user = Some(arguments.next().ok_or("--user needs a uuid argument")?),
            "--email" => email = Some(arguments.next().ok_or("--email needs an address")?),
            "--by" => granted_by = Some(arguments.next().ok_or("--by needs a name argument")?),
            _ => positional.push(argument),
        }
    }
    let db_url = positional.first().cloned().ok_or_else(usage_error)?;
    let verb = positional.get(1).cloned().ok_or_else(usage_error)?;
    let subject = || -> Result<Subject, Box<dyn std::error::Error>> {
        match (&user, &email) {
            (Some(raw), None) => Ok(Subject::User(user_from_uuid(raw)?)),
            (None, Some(address)) => Ok(Subject::Email(address.clone())),
            (None, None) => Err("name the operator with --user <uuid> or --email <address>".into()),
            (Some(_), Some(_)) => {
                Err("--user and --email name one operator two ways; give one".into())
            }
        }
    };
    let command = match verb.as_str() {
        "grant" => Command::Grant {
            subject: subject()?,
            granted_by: granted_by.unwrap_or_else(|| DEFAULT_GRANTED_BY.to_owned()),
        },
        "revoke" => Command::Revoke {
            subject: subject()?,
        },
        "list" => Command::List,
        other => {
            eprintln!("{USAGE}");
            return Err(format!("unknown command {other:?}").into());
        }
    };
    Ok(Invocation { db_url, command })
}

/// The subject to a user id, refusing rather than guessing when no such user
/// exists. An address that names nobody is a typo far more often than it is a
/// user who has not signed in yet, and creating the row here would make the
/// typo permanent.
async fn resolve(
    sessions: &SessionRepo,
    subject: Subject,
) -> Result<UserId, Box<dyn std::error::Error>> {
    match subject {
        Subject::User(user) => Ok(user),
        Subject::Email(address) => sessions
            .user_by_email(&address)
            .await?
            .ok_or_else(|| format!("no user holds the address {address}").into()),
    }
}

fn render(user: UserId) -> String {
    uuid::Uuid::from_bytes(user.0 .0).to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let invocation = parse_invocation()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&invocation.db_url)
        .await?;
    let operators = OperatorRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool);

    match invocation.command {
        Command::Grant {
            subject,
            granted_by,
        } => {
            let user = resolve(&sessions, subject).await?;
            operators.grant(user, &granted_by, wall_now()?).await?;
            println!("operator granted: {} (by {granted_by})", render(user));
        }
        Command::Revoke { subject } => {
            let user = resolve(&sessions, subject).await?;
            if !operators.revoke(user, wall_now()?).await? {
                return Err(format!("{} holds no active operator grant", render(user)).into());
            }
            println!("operator revoked: {}", render(user));
        }
        Command::List => {
            let records = operators.list().await?;
            if records.is_empty() {
                eprintln!("no operator has ever been granted");
            }
            for record in records {
                let standing = match record.revoked_at {
                    Some(at) => format!("revoked at {}", at.0),
                    None => "active".to_owned(),
                };
                println!(
                    "{}\t{}\t{standing}\tgranted at {} by {}",
                    render(record.user),
                    record.email,
                    record.granted_at.0,
                    record.granted_by,
                );
            }
        }
    }
    Ok(())
}
