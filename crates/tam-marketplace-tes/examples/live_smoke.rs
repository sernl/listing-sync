//! The operator's supervised live check, replacing the deleted spike's role.
//!
//! Usage, from the repo root with the operator's cookie jar:
//!   cargo run -p tam-marketplace-tes --example live_smoke -- <jar-path>
//!       preflight                       (default: create, probe-write, read, delete one ZZ draft)
//!       draft <pdf-path>                (full draft flow, then verified delete)
//!       publish <pdf-path> --yes-publish-live   (goes LIVE; founder deletes after verifying)

use std::io::Read as _;

use tam_marketplace::{FileContent, FileSource, FileSourceError, FormId, MarketplaceAdapter};
use tam_marketplace_tes::endpoints;
use tam_marketplace_tes::{ReqwestTransport, TesAdapter, TesSession};
use tam_types::{FileId, InventoryId, OrgId, Uuid};

struct OneFile(FileContent);

impl FileSource for OneFile {
    async fn fetch(&self, _file: FileId) -> Result<FileContent, FileSourceError> {
        Ok(self.0.clone())
    }
}

fn read_file(path: &str) -> Result<Vec<u8>, std::io::Error> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let jar_path = arguments.first().ok_or(
        "usage: live_smoke <jar-path> [preflight|draft <pdf>|publish <pdf> --yes-publish-live]",
    )?;
    let mut jar = String::new();
    std::fs::File::open(jar_path)?.read_to_string(&mut jar)?;
    let session = TesSession::from_netscape_jar(&jar)?;
    let transport = ReqwestTransport::new(&session)?;

    let mode = arguments.get(1).map_or("preflight", String::as_str);
    let org = OrgId(Uuid([0; 16]));

    match mode {
        "preflight" => {
            let adapter = TesAdapter::new(
                InventoryId::TesGb,
                transport,
                OneFile(FileContent {
                    file_name: String::new(),
                    content_type: String::new(),
                    bytes: Vec::new(),
                }),
            )?;
            let fingerprint = adapter
                .assert_form_schema(org, FormId(Uuid([1; 16])))
                .await
                .map_err(|error| format!("preflight failed: {error:?}"))?;
            eprintln!(
                "preflight ok; draft schema fingerprint {:02x?}",
                fingerprint.0 .0
            );
        }
        "draft" | "publish" => {
            let pdf_path = arguments
                .get(2)
                .ok_or("this mode needs a pdf path argument")?;
            let file = FileContent {
                file_name: "smoke.pdf".to_owned(),
                content_type: "application/pdf".to_owned(),
                bytes: read_file(pdf_path)?,
            };
            let adapter = TesAdapter::new(InventoryId::TesGb, transport, OneFile(file.clone()))?;
            let id = adapter
                .create_listing(&endpoints::probe_listing(), &[file])
                .await
                .map_err(|error| format!("draft flow failed: {error:?}"))?;
            eprintln!("draft created: id={}", id.0);
            if mode == "publish" {
                if arguments
                    .iter()
                    .any(|argument| argument == "--yes-publish-live")
                {
                    let state = adapter
                        .publish(id)
                        .await
                        .map_err(|error| format!("publish failed: {error:?}"))?;
                    eprintln!(
                        "PUBLISHED id={} draft={} — verify, then delete via the dashboard or delete mode",
                        id.0, state["draft"]
                    );
                } else {
                    eprintln!(
                        "refusing to publish without --yes-publish-live; draft id={} kept",
                        id.0
                    );
                }
            } else {
                adapter
                    .delete(id)
                    .await
                    .map_err(|error| format!("verified delete failed: {error:?}"))?;
                eprintln!("draft {} deleted and verified gone", id.0);
            }
        }
        other => return Err(format!("unknown mode {other:?}").into()),
    }
    Ok(())
}
