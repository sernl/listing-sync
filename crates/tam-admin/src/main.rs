//! The operator command-line tool: grants the platform-operator marking,
//! withdraws it, lists who holds it, and runs the deployment-time data
//! normalizations a migration cannot express in SQL.
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
//!        tam-admin <db-url> backfill-workflow-owners
//!        tam-admin guides seed --dir <path> [--dry-run] [--db <url>]
//!                              [--user <uuid> | --email <address>]
//!        tam-admin covers repair --db <url> [--org <uuid>] [--dry-run]
//!                                [--blob-kek-path <path> (--blob-store-root <dir> |
//!                                 --blob-store-s3 <endpoint> --blob-store-bucket <name>
//!                                 --blob-store-credentials <path> [--blob-store-region <r>])]
//!
//! The guide seed is the one command whose database is optional: `--dry-run`
//! reads the corpus, prints what it would write and never connects, so a
//! build lane can check the files parse without a server.
//!
//! `covers repair --dry-run` reads the catalogue and counts; only the real run
//! needs the key and the object store, because only it reads files and
//! writes covers.

#![forbid(unsafe_code)]

mod covers_repair;
mod guides_seed;

use std::path::PathBuf;

use tam_blob_store::BackendFlags;
use tam_storage::{GuideRepo, OperatorRepo, SessionRepo, SyncRequestRepo};
use tam_types::{OrgId, Timestamp, UserId, Uuid};

const USAGE: &str =
    "usage: tam-admin <db-url> grant  (--user <uuid> | --email <address>) [--by <who>]\n\
                     \x20      tam-admin <db-url> revoke (--user <uuid> | --email <address>)\n\
                     \x20      tam-admin <db-url> list\n\
                     \x20      tam-admin <db-url> backfill-workflow-owners\n\
                     \x20      tam-admin guides seed --dir <path> [--dry-run] [--db <url>]\n\
                     \x20                            [--user <uuid> | --email <address>]\n\
                     \x20      tam-admin covers repair --db <url> [--org <uuid>] [--dry-run]\n\
                     \x20                              [--blob-kek-path <path> <blob-store flags>]";

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
    /// The deployment-time normalization that gives a legacy migration's
    /// event anchor the request it belongs to.
    ///
    /// Named for what it writes rather than for the release that needed it:
    /// an anchor minted before the import leg named its request carries no
    /// owner, so a deletion fences nothing and the device's next page walks
    /// through the hole. It takes no subject and no tenant -- it visits
    /// every organisation -- and it is idempotent, so the launcher may run
    /// it on every boot.
    BackfillWorkflowOwners,
    /// The help corpus, read from a directory of Markdown files and written
    /// into the guide tables.
    ///
    /// The author is who the working copy records as having last touched it.
    /// Nobody names one in the ordinary case, and the command then attributes
    /// the write to the first operator holding an active grant: a seed is a
    /// deployment step, and attributing it to a human who did not run it
    /// would be the tool inventing evidence, while attributing it to nobody
    /// is not available -- the column the write fills is not nullable.
    GuidesSeed {
        dir: PathBuf,
        dry_run: bool,
        author: Option<Subject>,
    },
    /// Redraws every thumbnail that is still a generated card, from the
    /// resource's first file where this server holds it. See
    /// [`covers_repair`].
    CoversRepair {
        org: Option<OrgId>,
        dry_run: bool,
        kek_path: Option<String>,
        backend: Option<tam_blob_store::BlobBackend>,
    },
}

struct Invocation {
    /// Absent where the command does not need one: `guides seed --dry-run`
    /// reads files and prints, and a database it never opens is not a
    /// missing argument.
    db_url: Option<String>,
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
    let mut dir = None;
    let mut db = None;
    let mut dry_run = false;
    let mut org = None;
    let mut kek_path = None;
    let mut blob_store = BackendFlags::default();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if blob_store.accept(&argument, &mut arguments)? {
            continue;
        }
        match argument.as_str() {
            "--user" => user = Some(arguments.next().ok_or("--user needs a uuid argument")?),
            "--email" => email = Some(arguments.next().ok_or("--email needs an address")?),
            "--by" => granted_by = Some(arguments.next().ok_or("--by needs a name argument")?),
            "--dir" => dir = Some(arguments.next().ok_or("--dir needs a directory argument")?),
            "--db" => db = Some(arguments.next().ok_or("--db needs a connection url")?),
            "--dry-run" => dry_run = true,
            "--org" => org = Some(arguments.next().ok_or("--org needs a uuid argument")?),
            "--blob-kek-path" => {
                kek_path = Some(
                    arguments
                        .next()
                        .ok_or("--blob-kek-path needs a path argument")?,
                );
            }
            _ => positional.push(argument),
        }
    }
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
    // Naming nobody is the ordinary case for a seed, so the two-ways refusal
    // still applies but the neither-way case is an absence rather than a
    // fault.
    let optional_subject = || -> Result<Option<Subject>, Box<dyn std::error::Error>> {
        match (&user, &email) {
            (None, None) => Ok(None),
            (Some(_) | None, _) => subject().map(Some),
        }
    };

    let head = positional.first().cloned().ok_or_else(usage_error)?;
    // The guide seed is addressed by name rather than by database, because
    // its dry run has no database to be addressed by.
    if head == "guides" {
        let verb = positional.get(1).map_or("", String::as_str);
        if verb != "seed" {
            eprintln!("{USAGE}");
            return Err(format!("unknown guides command {verb:?}").into());
        }
        let dir = dir.ok_or("name the corpus with --dir <path>")?;
        return Ok(Invocation {
            db_url: db,
            command: Command::GuidesSeed {
                dir: PathBuf::from(dir),
                dry_run,
                author: optional_subject()?,
            },
        });
    }

    if head == "covers" {
        let verb = positional.get(1).map_or("", String::as_str);
        if verb != "repair" {
            eprintln!("{USAGE}");
            return Err(format!("unknown covers command {verb:?}").into());
        }
        let org = org
            .map(|raw| uuid::Uuid::parse_str(&raw).map(|parsed| OrgId(Uuid(*parsed.as_bytes()))))
            .transpose()?;
        let backend = blob_store.resolve()?;
        if !dry_run && (kek_path.is_none() || backend.is_none()) {
            return Err(
                "a repair reads files and writes covers: give --blob-kek-path and the \
                        blob-store flags, or --dry-run"
                    .into(),
            );
        }
        return Ok(Invocation {
            db_url: db,
            command: Command::CoversRepair {
                org,
                dry_run,
                kek_path,
                backend,
            },
        });
    }

    let verb = positional.get(1).cloned().ok_or_else(usage_error)?;
    let command = match verb.as_str() {
        "grant" => Command::Grant {
            subject: subject()?,
            granted_by: granted_by.unwrap_or_else(|| DEFAULT_GRANTED_BY.to_owned()),
        },
        "revoke" => Command::Revoke {
            subject: subject()?,
        },
        "list" => Command::List,
        "backfill-workflow-owners" => Command::BackfillWorkflowOwners,
        other => {
            eprintln!("{USAGE}");
            return Err(format!("unknown command {other:?}").into());
        }
    };
    Ok(Invocation {
        db_url: Some(head),
        command,
    })
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

/// Who a guide seed records as the author when the operator named nobody:
/// whoever holds the oldest active operator grant.
async fn seeding_author(operators: &OperatorRepo) -> Result<UserId, Box<dyn std::error::Error>> {
    operators
        .list()
        .await?
        .into_iter()
        .find(|record| record.revoked_at.is_none())
        .map(|record| record.user)
        .ok_or_else(|| {
            "no operator holds an active grant; run `grant` first or name an author with --user <uuid>".into()
        })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let invocation = parse_invocation()?;

    // The one path that needs no server, answered before the connection is
    // attempted rather than after it fails.
    if let Command::GuidesSeed {
        dir,
        dry_run: true,
        author: _,
    } = &invocation.command
    {
        for file in guides_seed::read_directory(dir)? {
            println!("{}", file.summary());
        }
        return Ok(());
    }

    let db_url = invocation
        .db_url
        .ok_or("name the database with <db-url> or --db <url>")?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await?;
    let operators = OperatorRepo::new(pool.clone());
    let sessions = SessionRepo::new(pool.clone());

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
        Command::BackfillWorkflowOwners => {
            // Tenant by tenant, through the repository's own roster, because
            // the pass runs as `tam_app` under forced row-level security and
            // cannot scan across organisations at all. One failing tenant
            // stops the command: an inconsistency this pass refuses to guess
            // at is a deployment that must not continue, and the tenants it
            // already normalized keep their links.
            let requests = SyncRequestRepo::new(pool);
            let mut linked = 0_u64;
            let mut tenants = 0_u64;
            for org in requests.tenants().await? {
                linked += requests.normalize_migration_anchors(org).await?;
                tenants += 1;
            }
            println!("workflow owners backfilled: {linked} anchor(s) across {tenants} tenant(s)");
        }
        Command::GuidesSeed {
            dir,
            dry_run: _,
            author,
        } => {
            // The dry run returned before the pool was opened, so reaching
            // here means the corpus is being written.
            let author = match author {
                Some(subject) => resolve(&sessions, subject).await?,
                None => seeding_author(&operators).await?,
            };
            let files = guides_seed::read_directory(&dir)?;
            let guides = GuideRepo::new(pool.clone());
            let outcomes = guides_seed::seed(&guides, &files, author, wall_now()?).await?;
            println!("{}", guides_seed::report(&outcomes));
        }
        Command::CoversRepair {
            org,
            dry_run,
            kek_path,
            backend,
        } => {
            let tenants = match org {
                Some(org) => vec![org],
                None => tam_storage::OrgRepo::new(pool.clone()).tenants().await?,
            };
            let store = match (dry_run, kek_path, backend) {
                (false, Some(path), Some(backend)) => Some(covers_repair::Store {
                    kek: load_kek(&path)?,
                    backend,
                }),
                _ => None,
            };
            let tally = covers_repair::run(&pool, &tenants, store.as_ref(), wall_now()?).await?;
            println!("{}", tally.report(dry_run));
        }
    }
    Ok(())
}

/// The blob key-encryption key, read from a file the way `tam-server` reads
/// it.
fn load_kek(path: &str) -> Result<tam_secrets::Kek, Box<dyn std::error::Error>> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(tam_secrets::Kek::from_bytes(&bytes)?)
}
