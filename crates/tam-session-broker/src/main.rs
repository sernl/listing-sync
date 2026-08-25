//! The privilege boundary, as a process rather than a module.
//!
//! The broker will be the sole holder of the key-encryption key and will expose
//! exactly one narrow unix-socket call returning a driver endpoint for a
//! browser it launched and primed itself. It exists from the first commit
//! because that boundary cannot be introduced later without redesigning every
//! call site that holds plaintext session material.
//!
//! Today it binds the socket and closes every connection it accepts, so the
//! address, the ownership and the lifecycle are settled before there is any
//! secret behind them. It serves no HTTP and links no web framework.

#![forbid(unsafe_code)]

use std::{
    io,
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
};

use tokio::net::UnixListener;

/// Under `.dev/`, which is git-ignored, so a development run leaves no socket
/// in the tree.
const DEFAULT_SOCKET_PATH: &str = "./.dev/tam-session-broker.sock";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = socket_path();
    let listener = bind(&path)?;
    assert!(
        path.exists(),
        "the socket node must exist once bind returned: {}",
        path.display()
    );
    eprintln!("tam-session-broker listening on {}", path.display());

    accept_until_shutdown(listener).await;
    Ok(())
}

/// The first positional argument, or the development default. Configuration is
/// read from the command line rather than the environment, which the lint table
/// bans outside the one crate that will own it.
fn socket_path() -> PathBuf {
    let argument = std::env::args().nth(1);
    PathBuf::from(argument.as_deref().unwrap_or(DEFAULT_SOCKET_PATH))
}

fn bind(path: &Path) -> io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    clear_stale_socket(path)?;
    UnixListener::bind(path)
}

/// A socket file outlives the process that bound it, so a restart finds its own
/// corpse and `bind` fails with `AddrInUse`. Only a socket is removed: anything
/// else at that path is someone's data and a mistyped argument.
fn clear_stale_socket(path: &Path) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_socket() {
        std::fs::remove_file(path)
    } else {
        Err(io::Error::other(format!(
            "{} exists and is not a socket, so it will not be removed",
            path.display()
        )))
    }
}

async fn accept_until_shutdown(listener: UnixListener) {
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((connection, _peer)) => drop(connection),
                Err(error) => eprintln!("tam-session-broker: accept failed: {error}"),
            },
            signal = tokio::signal::ctrl_c() => {
                if let Err(error) = signal {
                    eprintln!("tam-session-broker: cannot wait on ctrl-c, shutting down now: {error}");
                }
                break;
            }
        }
    }
}
