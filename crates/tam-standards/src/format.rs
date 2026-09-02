//! The on-disk format: one JSON Lines file of nodes per framework, and one
//! `manifest.json` carrying the provenance those nodes join against.
//!
//! Splitting it this way keeps the node file exactly what it claims to be --
//! one node per line, sorted, nothing else -- while the per-set subject,
//! grade band, licence and source URL live once in the manifest rather than
//! nineteen thousand times in the rows. A node names its set by index into
//! that framework's `sets`, and `StandardsDocument` is the join.
//!
//! Determinism is a property of the writer, not a convention: the rows are
//! sorted by a total key, every optional field is omitted rather than written
//! null, and no map is serialised from an unordered container. Two runs over
//! the same upstream bytes therefore produce the same file, which is what
//! makes the manifest's content hash worth checking.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::model::{Framework, Jurisdiction, Licence, SetRef, StandardNode};

/// The format tag written into every manifest, bumped when a reader would
/// misread an older file rather than whenever a field is added.
pub const FORMAT: &str = "tam-standards/1";

/// The whole provenance document: what was ingested, from where, when, under
/// which licence, and what the bytes hash to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub generated_at: String,
    pub frameworks: Vec<FrameworkManifest>,
}

impl Manifest {
    pub fn framework(&self, framework: Framework) -> Option<&FrameworkManifest> {
        self.frameworks
            .iter()
            .find(|entry| entry.framework == framework)
    }
}

/// One framework's ingest: its file, its counts, and every set behind them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkManifest {
    pub framework: Framework,
    pub jurisdiction: Jurisdiction,
    /// RFC 3339, in UTC. The fetch date a licence notice and a staleness
    /// check both refer to.
    pub fetched_at: String,
    /// The mirror the rows came from, named so a reconciliation item raised
    /// against a row can say which trust level it carries.
    pub source: String,
    pub source_api: String,
    /// File name relative to the manifest, including any compression suffix.
    pub file: String,
    pub file_bytes: u64,
    /// SHA-256 of the file's bytes exactly as committed.
    pub file_sha256: String,
    pub node_count: usize,
    pub distinct_code_count: usize,
    pub addressable_count: usize,
    /// Every distinct licence declared across this framework's sets.
    pub licences: Vec<Licence>,
    pub sets: Vec<SetRef>,
}

/// A framework's nodes joined to the provenance that describes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardsDocument {
    pub manifest: FrameworkManifest,
    pub nodes: Vec<StandardNode>,
}

impl StandardsDocument {
    pub fn framework(&self) -> Framework {
        self.manifest.framework
    }

    /// The set a node was mirrored from, and the source of its subject, grade
    /// band, licence and owner document.
    pub fn set_of(&self, node: &StandardNode) -> Option<&SetRef> {
        self.manifest.sets.get(node.set)
    }

    pub fn subject_of(&self, node: &StandardNode) -> Option<&str> {
        self.set_of(node).map(|set| set.subject.as_str())
    }

    /// Nodes a seller can tag, in file order.
    pub fn addressable(&self) -> impl Iterator<Item = &StandardNode> {
        self.nodes.iter().filter(|node| node.addressable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The manifest names a format this reader does not implement.
    UnknownFormat {
        found: String,
    },
    /// The manifest carries no entry for the framework asked for.
    MissingFramework {
        framework: Framework,
    },
    /// A line did not parse, named by its one-based number so the file can be
    /// opened at it.
    Line {
        line: usize,
        message: String,
    },
    Manifest {
        message: String,
    },
    /// The committed bytes are not the bytes the manifest was written for.
    /// Recorded rather than repaired: a hash mismatch means the file and its
    /// provenance disagree, and guessing which is right is how a wrong
    /// statement reaches a seller's listing.
    ContentHash {
        framework: Framework,
        expected: String,
        found: String,
    },
    /// A node named a set index the manifest does not have.
    DanglingSet {
        line: usize,
        set: usize,
    },
    NodeCount {
        declared: usize,
        found: usize,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::UnknownFormat { found } => {
                write!(
                    formatter,
                    "unknown standards format `{found}`, expected `{FORMAT}`"
                )
            }
            LoadError::MissingFramework { framework } => {
                write!(formatter, "the manifest carries no entry for {framework}")
            }
            LoadError::Line { line, message } => {
                write!(formatter, "line {line}: {message}")
            }
            LoadError::Manifest { message } => write!(formatter, "manifest: {message}"),
            LoadError::ContentHash {
                framework,
                expected,
                found,
            } => write!(
                formatter,
                "{framework}: committed bytes hash to {found}, manifest declares {expected}"
            ),
            LoadError::DanglingSet { line, set } => {
                write!(
                    formatter,
                    "line {line}: set index {set} is not in the manifest"
                )
            }
            LoadError::NodeCount { declared, found } => {
                write!(
                    formatter,
                    "manifest declares {declared} nodes, the file holds {found}"
                )
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// Lowercase hex of the SHA-256 of `bytes`.
pub fn content_hash(bytes: &[u8]) -> String {
    const HEX: [u8; 16] = *b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut rendered = String::with_capacity(64);
    for byte in digest {
        rendered.push(char::from(HEX[usize::from(byte >> 4)]));
        rendered.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    rendered
}

pub fn parse_manifest(json: &str) -> Result<Manifest, LoadError> {
    let manifest: Manifest = serde_json::from_str(json).map_err(|error| LoadError::Manifest {
        message: error.to_string(),
    })?;
    if manifest.format != FORMAT {
        return Err(LoadError::UnknownFormat {
            found: manifest.format,
        });
    }
    Ok(manifest)
}

/// Read one framework's nodes and check them against the manifest that
/// describes them.
///
/// `bytes` is the file exactly as committed, because the hash the manifest
/// declares is over those bytes and nothing else. Every check the manifest
/// makes possible runs here rather than at the caller: the content hash, the
/// node count, and every set index a node names. A caller that skipped one of
/// them would be holding rows whose provenance it could not state.
pub fn load_framework(
    manifest: &Manifest,
    framework: Framework,
    bytes: &[u8],
) -> Result<StandardsDocument, LoadError> {
    let entry = manifest
        .framework(framework)
        .ok_or(LoadError::MissingFramework { framework })?;

    let found = content_hash(bytes);
    if found != entry.file_sha256 {
        return Err(LoadError::ContentHash {
            framework,
            expected: entry.file_sha256.clone(),
            found,
        });
    }

    let text = std::str::from_utf8(bytes).map_err(|error| LoadError::Line {
        line: 0,
        message: error.to_string(),
    })?;

    let mut nodes = Vec::with_capacity(entry.node_count);
    for (offset, line) in text.lines().enumerate() {
        let number = offset + 1;
        if line.is_empty() {
            continue;
        }
        let node: StandardNode = serde_json::from_str(line).map_err(|error| LoadError::Line {
            line: number,
            message: error.to_string(),
        })?;
        if node.set >= entry.sets.len() {
            return Err(LoadError::DanglingSet {
                line: number,
                set: node.set,
            });
        }
        nodes.push(node);
    }

    if nodes.len() != entry.node_count {
        return Err(LoadError::NodeCount {
            declared: entry.node_count,
            found: nodes.len(),
        });
    }

    Ok(StandardsDocument {
        manifest: entry.clone(),
        nodes,
    })
}

/// The total order the rows are written in.
///
/// Code first because that is how a reviewer reads the file and how a diff
/// between two ingests lines up; the mirror's own identifier breaks every tie,
/// so the order is total and no two runs can disagree. Nodes without a code
/// sort last as a block rather than being scattered through the coded ones.
fn sort_key(node: &StandardNode) -> (bool, &str, &str) {
    (
        node.code.is_none(),
        node.code.as_deref().unwrap_or(""),
        node.source_guid.as_str(),
    )
}

/// Render a framework's nodes as the committed file: sorted, one per line,
/// newline-terminated.
pub fn render_jsonl(nodes: &mut [StandardNode]) -> Result<String, serde_json::Error> {
    nodes.sort_by(|left, right| sort_key(left).cmp(&sort_key(right)));
    let mut rendered = String::new();
    for node in nodes.iter() {
        rendered.push_str(&serde_json::to_string(node)?);
        rendered.push('\n');
    }
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grades::GradeBand;

    fn node(code: Option<&str>, guid: &str) -> StandardNode {
        StandardNode {
            set: 0,
            source_guid: guid.to_owned(),
            asn_identifier: None,
            parent_guid: None,
            depth: 0,
            code: code.map(str::to_owned),
            alt_code: None,
            node_type: None,
            statement: "text".to_owned(),
            hierarchy_path: Vec::new(),
            uri: None,
            addressable: true,
        }
    }

    fn set_ref() -> SetRef {
        SetRef {
            id: "S1".to_owned(),
            title: None,
            subject: "Mathematics".to_owned(),
            normalized_subject: None,
            grade_band: GradeBand::default(),
            source_url: "https://example.invalid/S1".to_owned(),
            document: None,
            licence: None,
        }
    }

    fn manifest_for(nodes: &[StandardNode], bytes: &[u8]) -> Manifest {
        Manifest {
            format: FORMAT.to_owned(),
            generated_at: "2026-09-03T00:00:00Z".to_owned(),
            frameworks: vec![FrameworkManifest {
                framework: Framework::Ccss,
                jurisdiction: Jurisdiction::Us,
                fetched_at: "2026-09-03T00:00:00Z".to_owned(),
                source: "commonstandardsproject".to_owned(),
                source_api: "https://api.commonstandardsproject.com".to_owned(),
                file: "ccss.jsonl".to_owned(),
                file_bytes: u64::try_from(bytes.len()).expect("test file fits a u64"),
                file_sha256: content_hash(bytes),
                node_count: nodes.len(),
                distinct_code_count: nodes.len(),
                addressable_count: nodes.len(),
                licences: Vec::new(),
                sets: vec![set_ref()],
            }],
        }
    }

    #[test]
    fn rendering_is_stable_under_input_order() {
        let mut one = vec![
            node(Some("B"), "g2"),
            node(Some("A"), "g1"),
            node(None, "g3"),
        ];
        let mut two = vec![
            node(None, "g3"),
            node(Some("A"), "g1"),
            node(Some("B"), "g2"),
        ];
        let rendered_one = render_jsonl(&mut one).expect("render");
        let rendered_two = render_jsonl(&mut two).expect("render");
        assert_eq!(
            rendered_one, rendered_two,
            "two input orders must render identically"
        );
        let lines: Vec<&str> = rendered_one.lines().collect();
        assert!(
            lines[2].contains("g3"),
            "uncoded rows sort last, got {lines:?}"
        );
    }

    #[test]
    fn a_rendered_file_round_trips_against_its_manifest() {
        let mut nodes = vec![node(Some("A"), "g1"), node(Some("B"), "g2")];
        let rendered = render_jsonl(&mut nodes).expect("render");
        let manifest = manifest_for(&nodes, rendered.as_bytes());
        let document =
            load_framework(&manifest, Framework::Ccss, rendered.as_bytes()).expect("load");
        assert_eq!(
            document.nodes, nodes,
            "the loaded rows are the rendered rows"
        );
        assert_eq!(document.subject_of(&document.nodes[0]), Some("Mathematics"));
    }

    #[test]
    fn a_byte_changed_after_the_manifest_was_written_is_refused() {
        let mut nodes = vec![node(Some("A"), "g1")];
        let rendered = render_jsonl(&mut nodes).expect("render");
        let manifest = manifest_for(&nodes, rendered.as_bytes());
        let tampered = rendered.replace("\"text\"", "\"TEXT\"");
        let error = load_framework(&manifest, Framework::Ccss, tampered.as_bytes())
            .expect_err("a hash mismatch must not load");
        assert!(
            matches!(error, LoadError::ContentHash { .. }),
            "expected a content-hash error, got {error:?}"
        );
    }

    #[test]
    fn a_node_naming_a_set_the_manifest_lacks_is_refused() {
        let mut nodes = vec![node(Some("A"), "g1")];
        nodes[0].set = 7;
        let rendered = render_jsonl(&mut nodes).expect("render");
        let manifest = manifest_for(&nodes, rendered.as_bytes());
        let error = load_framework(&manifest, Framework::Ccss, rendered.as_bytes())
            .expect_err("a dangling set index must not load");
        assert!(
            matches!(error, LoadError::DanglingSet { set: 7, .. }),
            "expected a dangling-set error, got {error:?}"
        );
    }

    #[test]
    fn a_manifest_from_a_future_format_is_refused_rather_than_guessed_at() {
        let error =
            parse_manifest(r#"{"format":"tam-standards/9","generated_at":"x","frameworks":[]}"#)
                .expect_err("an unknown format must not parse");
        assert!(
            matches!(error, LoadError::UnknownFormat { .. }),
            "expected an unknown-format error, got {error:?}"
        );
    }
}
