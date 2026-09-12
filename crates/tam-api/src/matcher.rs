//! Whether two resources are the same resource.
//!
//! A two-stage pipeline, which is the standard shape for entity resolution: a
//! cheap blocker narrows the org's catalogue to the candidates worth looking
//! at, and a fused scorer decides them. The blocker is three indexed reads
//! (`tam_storage::FingerprintRepo`); the scorer is [`score`], which is pure
//! and takes no database.
//!
//! Three rules the whole module is built to keep.
//!
//! **One organisation only.** Matching across tenants would leak one seller's
//! catalogue into another's, and per-tenant blob encryption makes a global
//! digest table an existence oracle besides.
//!
//! **Cross-marketplace only.** Henzinger's evaluation found near-duplicate
//! detection works badly *within* one site, which is exactly "two listings by
//! the same seller on the same marketplace" — and that is also the case a
//! seller may have made deliberately. A product mapped onto no marketplace is
//! comparable with anything, because it is on no side to be the same side of.
//!
//! **Frequency weighting is not optional.** A seller's `Terms of Use.pdf` is
//! byte-identical across their entire range, so an unweighted digest equality
//! would merge their whole store. The same holds for a title: a range all
//! called "reading comprehension" has told us nothing by agreeing on it.
//!
//! The asymmetry between the outcomes is deliberate and is the reason the
//! system may only ever decide `same`, only on a decisive layer, and only
//! with no negative firing. A missed duplicate costs the seller one review
//! click; a false merge collapses two real products and the next publish
//! overwrites a live listing with the wrong resource. Henzinger's best *fused*
//! precision on web pages was 0.79 — one wrong merge in five.

use std::collections::HashMap;

use tam_fingerprint::{hamming, minhash_jaccard, token_jaccard};
use tam_storage::{DecidedBy, Evidence, EvidenceUnit, MatchLayer, Polarity, Verdict};
use tam_types::{ContentHash, Marketplace, Money, OrgId, ProductId};

use crate::error::APIError;
use crate::jobs::storage_fault;
use crate::AppState;

/// The fused score above which the seller is asked.
///
/// Three, which the weights below make mean "two independent moderate signals,
/// or one strong one". Below it nothing is written at all: silence is not a
/// merge, and a `different` row for every pair the blocker touched would fill
/// the table with answers nobody gave.
pub const REVIEW_FLOOR: f32 = 3.0;

/// The smallest file an exact digest match is decisive over.
///
/// Fifty kilobytes. Below it a byte-identical file is as likely to be a
/// one-page cover letter, a licence note or an empty template as it is to be
/// the resource, and those are precisely the files a seller reuses across a
/// catalogue.
pub const L1_MIN_BYTES: u64 = 50 * 1024;

/// How many products may carry a value before agreeing on it earns nothing.
///
/// Two, from the research: a digest or a title held by three products in one
/// organisation is that seller's boilerplate rather than a resource's
/// identity.
pub const COMMON_VALUE_MAX: u32 = 2;

/// One side of a pair, with everything the layers read and nothing else.
///
/// The same type for a product already in the catalogue and for a resource a
/// device has just described, which is the point: the scorer cannot treat one
/// as privileged, and the one-sided cases — a TPT read that names no file at
/// all — are absences rather than a second shape.
#[derive(Debug, Clone, Default)]
pub struct Side {
    pub title: String,
    pub title_norm: String,
    pub price: Option<Money>,
    pub text: Option<TextFacts>,
    pub page_count: Option<u32>,
    /// Present only for an image payload, which is where the producer computes
    /// it: `render::cover` gives every non-image payload a flat placeholder
    /// card, so a pHash over a PDF's cover compares constants. The absence is
    /// what keeps L3 from firing on them.
    pub cover_phash: Option<u64>,
    pub subjects: Vec<uuid::Uuid>,
    pub grade_low: Option<i16>,
    pub grade_high: Option<i16>,
    pub files: Vec<SideFile>,
}

/// The text sketch, as the layers read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFacts {
    pub simhash: u64,
    pub minhash: [u32; 128],
    pub extracted_chars: u32,
}

/// One payload entry, digested after the unwrap decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideFile {
    pub digest: ContentHash,
    pub byte_len: u64,
    pub name: Option<String>,
}

/// How common each value is inside this organisation.
#[derive(Debug, Clone, Default)]
pub struct Frequencies {
    pub digests: HashMap<[u8; 32], u32>,
    pub title: u32,
}

/// What the layers found, fused.
#[derive(Debug, Clone)]
pub struct Score {
    pub log_odds: f32,
    pub evidence: Vec<Evidence>,
    /// The heaviest positive layer that fired, which is what the console's one
    /// sentence is generated from. `None` where nothing did.
    pub winning_layer: Option<MatchLayer>,
    /// Whether a strong negative fired, which no amount of positive evidence
    /// overrides for an automatic merge.
    pub negative: bool,
    /// Whether the decisive layer fired.
    pub decisive: bool,
}

/// The three outcomes, which are not three points on one scale: the middle one
/// is a question and the outer two are answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Merge without asking. Only reachable on a decisive layer.
    Same,
    /// Put it to the seller.
    Ask,
    /// Distinct, and nothing is written: silence is not a merge.
    Different,
}

/// What an exact payload digest is worth, and the one place the number lives.
///
/// Read by the scorer and by the commit's own revalidation, which re-asks this
/// layer against what is committed now: a pair decided at commit time has to
/// be recorded with the same weight and the same evidence the matcher would
/// have recorded, or the console's one sentence would be generated from two
/// different definitions of one finding.
pub const L1_LOG_ODDS: f32 = 6.0;

/// The evidence an exact payload digest is, in the shape the verdict stores.
#[must_use]
pub fn exact_file_evidence(byte_len: u64, name: Option<String>) -> Evidence {
    Evidence {
        layer: MatchLayer::L1,
        polarity: Polarity::Positive,
        measure: byte_measure(byte_len),
        unit: EvidenceUnit::Bytes,
        observed_in: name,
    }
}

/// Scores one pair. Pure: no database, no clock, no organisation.
#[must_use]
pub fn score(lhs: &Side, rhs: &Side, frequency: &Frequencies, trigram: f32) -> Score {
    let mut evidence: Vec<Evidence> = Vec::new();
    let mut log_odds = 0.0_f32;
    let mut negative = false;
    let mut decisive = false;

    // The files worth comparing at all: everything this seller's catalogue is
    // not full of. Computed once and used by both file layers, because the
    // frequency rule is about the value rather than about the layer -- a
    // `Terms of Use.pdf` on every product tells us nothing by matching
    // exactly, and it tells us nothing by being in both sets either. Applying
    // it to L1 alone would move the merge-every-product failure from one layer
    // to the next one down.
    let left_files = comparable(&lhs.files, frequency);
    let right_files = comparable(&rhs.files, frequency);

    // ---- L1: one payload entry, byte for byte.
    if let Some((file, other)) = exact_entry(&left_files, &right_files) {
        decisive = true;
        log_odds += L1_LOG_ODDS;
        evidence.push(exact_file_evidence(
            file.byte_len,
            file.name.clone().or_else(|| other.name.clone()),
        ));
    } else if let Some(overlap) = digest_overlap(&left_files, &right_files) {
        // ---- L1b: the sets overlap, which is a Tes multi-file bundle against
        // a TPT single file. Only where L1 did not fire, because the two are
        // one measurement of the same thing at two resolutions.
        if overlap >= 0.6 {
            log_odds += 4.0;
            evidence.push(Evidence {
                layer: MatchLayer::L1b,
                polarity: Polarity::Positive,
                measure: overlap,
                unit: EvidenceUnit::Jaccard,
                observed_in: None,
            });
        }
    }

    // ---- L2: the extracted text. Survives a re-export, which is the real
    // case: re-exporting changes bytes and leaves words alone.
    let mut l2_strong = false;
    if let (Some(left), Some(right)) = (lhs.text, rhs.text) {
        if left.extracted_chars > 0 && right.extracted_chars > 0 {
            let jaccard = minhash_jaccard(&left.minhash, &right.minhash);
            if jaccard >= 0.9 {
                l2_strong = true;
                log_odds += 4.0;
            } else if jaccard >= 0.6 {
                log_odds += 2.0;
            }
            if jaccard >= 0.6 {
                evidence.push(Evidence {
                    layer: MatchLayer::L2,
                    polarity: Polarity::Positive,
                    measure: jaccard,
                    unit: EvidenceUnit::Jaccard,
                    observed_in: None,
                });
            }
        }
    }

    // ---- L3: the cover picture, for image payloads only.
    if let (Some(left), Some(right)) = (lhs.cover_phash, rhs.cover_phash) {
        let distance = hamming(left, right);
        if distance <= 6 {
            log_odds += 1.5;
            evidence.push(Evidence {
                layer: MatchLayer::L3,
                polarity: Polarity::Positive,
                #[allow(clippy::cast_precision_loss)]
                measure: distance as f32,
                unit: EvidenceUnit::Hamming,
                observed_in: None,
            });
        }
    }

    // ---- L4: the title, frequency-weighted.
    if frequency.title <= COMMON_VALUE_MAX {
        let tokens = token_jaccard(&lhs.title_norm, &rhs.title_norm);
        let (weight, measure, unit) = if tokens >= 0.8 {
            (2.0, tokens, EvidenceUnit::Jaccard)
        } else if tokens >= 0.6 {
            (1.0, tokens, EvidenceUnit::Jaccard)
        } else if trigram >= 0.5 {
            (1.0, trigram, EvidenceUnit::Ratio)
        } else {
            (0.0, tokens, EvidenceUnit::Jaccard)
        };
        if weight > 0.0 {
            log_odds += weight;
            evidence.push(Evidence {
                layer: MatchLayer::L4,
                polarity: Polarity::Positive,
                measure,
                unit,
                observed_in: None,
            });
        }
    }

    // ---- L5: corroboration, and the one strong negative in the whole
    // matcher.
    let mut corroboration = 0.0_f32;
    let mut corroborated = false;
    if grades_overlap(lhs, rhs) {
        corroboration += 0.5;
        corroborated = true;
    }
    if shares_subject(lhs, rhs) {
        corroboration += 0.5;
        corroborated = true;
    }
    if price_within(lhs, rhs) {
        corroboration += 0.5;
        corroborated = true;
    }
    let mut pages_equal = false;
    if let (Some(left), Some(right)) = (lhs.page_count, rhs.page_count) {
        if left == right {
            pages_equal = true;
            corroboration += 1.0;
            corroborated = true;
        } else {
            let (small, large) = if left < right {
                (left, right)
            } else {
                (right, left)
            };
            #[allow(clippy::cast_precision_loss)]
            let ratio = f32::from(u16::try_from(large).unwrap_or(u16::MAX))
                / f32::from(u16::try_from(small.max(1)).unwrap_or(1));
            if ratio > 1.2 {
                negative = true;
                log_odds -= 4.0;
                evidence.push(Evidence {
                    layer: MatchLayer::L5,
                    polarity: Polarity::Negative,
                    measure: ratio,
                    unit: EvidenceUnit::Ratio,
                    observed_in: None,
                });
            }
        }
    }
    if corroborated {
        log_odds += corroboration;
        evidence.push(Evidence {
            layer: MatchLayer::L5,
            polarity: Polarity::Positive,
            measure: corroboration,
            unit: EvidenceUnit::Count,
            observed_in: None,
        });
    }

    // The decisive conjunct the design states: L2 alone is strong but not
    // decisive, and what makes it decisive is an equal page count. Both page
    // counts must be present for that, which is the correction the seam report
    // records: a TPT read carries no page count, so a TPT-sourced pair never
    // reaches this and is always asked.
    let decisive = decisive || (l2_strong && pages_equal);

    let winning_layer = MatchLayer::ALL.into_iter().find(|layer| {
        evidence
            .iter()
            .any(|found| found.layer == *layer && found.polarity == Polarity::Positive)
    });

    Score {
        log_odds,
        evidence,
        winning_layer,
        negative,
        decisive,
    }
}

/// The outcome a score earns.
#[must_use]
pub fn outcome_of(score: &Score) -> Outcome {
    if score.decisive && !score.negative {
        return Outcome::Same;
    }
    if score.log_odds >= REVIEW_FLOOR {
        return Outcome::Ask;
    }
    Outcome::Different
}

/// The one sentence the console shows, generated from the layer that carried
/// the decision rather than re-scored on read.
///
/// Never a score. A seller asked to decide whether two resources are the same
/// thing is owed the reason, and "0.79" is not one.
#[must_use]
pub fn sentence(score: &Score) -> String {
    let Some(layer) = score.winning_layer else {
        return "These two look related.".to_owned();
    };
    sentence_for(layer, &score.evidence)
}

/// The same sentence, from the stored row rather than from a live score.
///
/// One function with two callers rather than two that agree today: the review
/// card is drawn from `duplicate_verdict` and `duplicate_evidence` long after
/// the score that produced them, and a second copy of the wording is how the
/// card and the ledger come to say different things about one pair.
#[must_use]
pub fn sentence_for(layer: MatchLayer, evidence: &[Evidence]) -> String {
    let found = evidence
        .iter()
        .find(|found| found.layer == layer && found.polarity == Polarity::Positive);
    match layer {
        MatchLayer::L1 => {
            let name = found
                .and_then(|found| found.observed_in.clone())
                .unwrap_or_else(|| "the same file".to_owned());
            let size = found.map_or_else(String::new, |found| {
                format!(", {}", megabytes(found.measure))
            });
            format!("The same file, byte for byte: {name}{size}.")
        }
        MatchLayer::L1b => "Most of the files match.".to_owned(),
        MatchLayer::L2 => {
            let percent = found.map_or(0, |found| percent_of(found.measure));
            format!("The text of both PDFs is {percent}% the same.")
        }
        MatchLayer::L3 => "The cover pictures look the same.".to_owned(),
        MatchLayer::L4 => "Nearly the same title.".to_owned(),
        MatchLayer::L5 => "Same grades and subject, similar price.".to_owned(),
    }
}

/// One pair the matcher raised, ready to be written and shown.
#[derive(Debug, Clone)]
pub struct Raised {
    pub other: ProductId,
    pub verdict: Verdict,
    pub decided_by: DecidedBy,
    pub layer: MatchLayer,
    pub log_odds: f32,
    pub evidence: Vec<Evidence>,
    pub sentence: String,
}

/// What the matcher concluded about one described resource.
#[derive(Debug, Clone, Default)]
pub struct Match {
    /// Pairs the system decided `same` on its own, on a decisive layer.
    pub merged: Vec<Raised>,
    /// Pairs put to the seller.
    pub asked: Vec<Raised>,
}

impl Match {
    /// Whether this resource owes the seller a question.
    #[must_use]
    pub fn needs_review(&self) -> bool {
        !self.asked.is_empty()
    }
}

/// One question put to the matcher.
///
/// A struct rather than eight arguments, and the only way in: every caller
/// scores inside its own guarded transaction, because a verdict is acted on
/// under the organisation's catalogue lock and a merge is irreversible for
/// thirty days. A pool-owned form existed and was deleted with the page
/// path's pre-lock scoring; nothing should be able to ask this question
/// outside the transaction that writes the answer.
pub struct Asking<'a> {
    pub state: &'a AppState,
    pub org: OrgId,
    pub version: i16,
    pub subject: &'a Side,
    pub subject_product: ProductId,
    pub marketplace: Option<Marketplace>,
    pub bands: Option<[i16; 4]>,
    pub reviews: bool,
}

/// Blocks, scores and decides one resource inside a transaction the caller
/// owns.
///
/// The commit calls this under the organisation's catalogue lock, and that is
/// the whole reason it exists: a decision the matcher reached when a page
/// landed is re-asked against the catalogue as it stands at the instant the
/// resource is about to be created, so a product another source committed in
/// the meantime is seen — by every layer, with every exclusion, rather than
/// by a digest comparison standing in for the scorer.
pub async fn match_one_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    asking: &Asking<'_>,
) -> Result<Match, APIError> {
    let Asking {
        state,
        org,
        version,
        subject,
        subject_product,
        marketplace,
        bands,
        reviews,
    } = *asking;
    let digests: Vec<ContentHash> = subject.files.iter().map(|file| file.digest).collect();

    // The third blocking key: products already carrying one of these digests.
    // Read first because it feeds the candidate query, so a pair that agrees
    // on bytes and on nothing else is still scored.
    let by_digest = tam_storage::products_by_digest_in(tx, org, &digests)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let mut also: Vec<ProductId> = Vec::new();
    for row in &by_digest {
        if !also.contains(&row.product) {
            also.push(row.product);
        }
    }

    let candidates =
        tam_storage::candidates_in(tx, org, version, bands, &subject.title_norm, &also)
            .await
            .map_err(|error| storage_fault(state, &error))?;
    let others: Vec<ProductId> = candidates
        .iter()
        .map(|candidate| candidate.product)
        .filter(|product| *product != subject_product)
        .collect();
    if others.is_empty() {
        return Ok(Match::default());
    }

    let metadata = tam_storage::metadata_for_in(tx, org, &others)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let files = tam_storage::digests_for_in(tx, org, &others)
        .await
        .map_err(|error| storage_fault(state, &error))?;
    let mut frequency = Frequencies {
        digests: tam_storage::digest_frequency_in(tx, org, &digests)
            .await
            .map_err(|error| storage_fault(state, &error))?
            .into_iter()
            .map(|(digest, held)| (digest.0, held))
            .collect(),
        title: tam_storage::title_frequency_in(tx, org, version, &subject.title_norm)
            .await
            .map_err(|error| storage_fault(state, &error))?,
    };
    // The subject's own title is one of the rows the count returned where it
    // already has a fingerprint, and one row is not a shared value. Counting
    // it would make a re-import of the same resource earn nothing from its own
    // title.
    frequency.title = frequency.title.saturating_sub(1);

    let pairs: Vec<(ProductId, ProductId)> = others
        .iter()
        .map(|other| (subject_product, *other))
        .collect();
    let answered = tam_storage::answered_pairs(tx, org, &pairs)
        .await
        .map_err(|error| storage_fault(state, &error))?;

    let mut found = Match::default();
    for candidate in &candidates {
        if candidate.product == subject_product {
            continue;
        }
        let meta = metadata
            .iter()
            .find(|meta| meta.product == candidate.product);
        // Cross-marketplace only. A candidate on the same shop this was read
        // from is not proposed, whatever it scores.
        if let (Some(marketplace), Some(meta)) = (marketplace, meta) {
            if meta.marketplaces.contains(&marketplace) {
                continue;
            }
        }
        // Asked already, in either direction. `answered` holds the pair in its
        // canonical order, so this is one lookup rather than two.
        let (lo, hi) = tam_storage::ordered_pair(subject_product, candidate.product);
        if answered
            .iter()
            .any(|(held_lo, held_hi, _)| *held_lo == lo && *held_hi == hi)
        {
            continue;
        }

        let other = side_of(candidate, meta, &files);
        let scored = score(subject, &other, &frequency, candidate.title_similarity);
        let Some(layer) = scored.winning_layer else {
            continue;
        };
        let raised = Raised {
            other: candidate.product,
            verdict: Verdict::Same,
            decided_by: DecidedBy::System,
            layer,
            log_odds: scored.log_odds,
            evidence: scored.evidence.clone(),
            sentence: sentence(&scored),
        };
        match outcome_of(&scored) {
            Outcome::Same => found.merged.push(raised),
            Outcome::Ask if reviews => found.asked.push(Raised {
                verdict: Verdict::Parked,
                decided_by: DecidedBy::Seller,
                ..raised
            }),
            // A plan without duplicate review treats every parked pair as
            // different, so nothing blocks. Nothing is written either: a
            // `different` the seller never gave would stop the question being
            // asked when they upgrade.
            Outcome::Ask | Outcome::Different => {}
        }
    }
    Ok(found)
}

/// One candidate, lifted into the scorer's own shape.
fn side_of(
    candidate: &tam_storage::CandidateSketch,
    meta: Option<&tam_storage::ProductMeta>,
    files: &[tam_storage::PayloadDigest],
) -> Side {
    Side {
        title: candidate.title.clone(),
        title_norm: candidate.title_norm.clone(),
        price: match candidate.price {
            tam_types::PriceIntent::Paid(money) => Some(money),
            tam_types::PriceIntent::Free => None,
        },
        text: candidate.text.as_ref().and_then(|text| {
            Some(TextFacts {
                simhash: u64::from_ne_bytes(text.simhash.to_ne_bytes()),
                minhash: tam_fingerprint::minhash_from_bytes(&text.minhash)?,
                extracted_chars: u32::try_from(text.extracted_chars).unwrap_or(0),
            })
        }),
        page_count: candidate
            .page_count
            .map(|count| u32::try_from(count).unwrap_or(0)),
        cover_phash: candidate
            .cover_phash
            .map(|phash| u64::from_ne_bytes(phash.to_ne_bytes())),
        subjects: meta.map(|meta| meta.subjects.clone()).unwrap_or_default(),
        grade_low: meta.and_then(|meta| meta.grade_low),
        grade_high: meta.and_then(|meta| meta.grade_high),
        files: files
            .iter()
            .filter(|file| file.product == candidate.product)
            .map(|file| SideFile {
                digest: file.digest,
                byte_len: file.byte_len,
                name: file.name.clone(),
            })
            .collect(),
    }
}

/// The entry both sides carry byte for byte, if it is one worth anything.
///
/// Three conjuncts, all three necessary: the digests and lengths agree, the
/// file is big enough not to be a licence note, and the digest is not one this
/// seller's catalogue is full of.
fn exact_entry<'a>(
    lhs: &[&'a SideFile],
    rhs: &[&'a SideFile],
) -> Option<(&'a SideFile, &'a SideFile)> {
    for left in lhs {
        for right in rhs {
            if left.digest != right.digest || left.byte_len != right.byte_len {
                continue;
            }
            // The size floor is L1's own: below it a byte-identical file is as
            // likely to be a cover letter or a licence note as it is to be the
            // resource, and those are exactly what a seller reuses.
            if left.byte_len < L1_MIN_BYTES {
                continue;
            }
            return Some((left, right));
        }
    }
    None
}

/// The entries whose agreement would mean something: everything this seller's
/// catalogue is not full of.
fn comparable<'a>(files: &'a [SideFile], frequency: &Frequencies) -> Vec<&'a SideFile> {
    files
        .iter()
        .filter(|file| {
            frequency.digests.get(&file.digest.0).copied().unwrap_or(1) <= COMMON_VALUE_MAX
        })
        .collect()
}

/// The Jaccard of the two digest sets, or `None` where either side has no
/// files at all — which is not an overlap of zero but an unmeasured layer.
fn digest_overlap(lhs: &[&SideFile], rhs: &[&SideFile]) -> Option<f32> {
    if lhs.is_empty() || rhs.is_empty() {
        return None;
    }
    let mut union: Vec<[u8; 32]> = Vec::new();
    for file in lhs {
        if !union.contains(&file.digest.0) {
            union.push(file.digest.0);
        }
    }
    let mut shared = 0_u32;
    for file in rhs {
        if union.contains(&file.digest.0) {
            shared = shared.saturating_add(1);
        } else {
            union.push(file.digest.0);
        }
    }
    #[allow(clippy::cast_precision_loss)]
    Some(shared as f32 / union.len() as f32)
}

fn grades_overlap(lhs: &Side, rhs: &Side) -> bool {
    match (lhs.grade_low, lhs.grade_high, rhs.grade_low, rhs.grade_high) {
        (Some(low), Some(high), Some(other_low), Some(other_high)) => {
            low <= other_high && other_low <= high
        }
        _ => false,
    }
}

fn shares_subject(lhs: &Side, rhs: &Side) -> bool {
    lhs.subjects
        .iter()
        .any(|subject| rhs.subjects.contains(subject))
}

/// Whether the two prices are within one and a half of each other.
///
/// Only where both are denominated the same way. Converting would put a rate
/// nobody measured inside a comparison, which is the same refusal
/// `CurrencyUnknown` exists for.
fn price_within(lhs: &Side, rhs: &Side) -> bool {
    let (Some(left), Some(right)) = (lhs.price, rhs.price) else {
        return false;
    };
    if left.currency() != right.currency() {
        return false;
    }
    let small = left.minor_units().min(right.minor_units()).max(1);
    let large = left.minor_units().max(right.minor_units());
    // Cross-multiplied rather than divided: `large / small <= 1.5` in integers
    // would floor the ratio and call 3 against 2 a match at exactly the
    // boundary while calling 30 against 19 one too, which is a different rule
    // from the one the design states.
    large.saturating_mul(2) <= small.saturating_mul(3)
}

/// The byte length, as the evidence column holds it.
#[allow(clippy::cast_precision_loss)]
fn byte_measure(byte_len: u64) -> f32 {
    byte_len as f32
}

/// The size a seller reads, from the stored measure.
fn megabytes(bytes: f32) -> String {
    let mb = bytes / (1024.0 * 1024.0);
    if mb < 0.1 {
        let kb = bytes / 1024.0;
        return format!("{kb:.0} KB");
    }
    format!("{mb:.1} MB")
}

fn percent_of(jaccard: f32) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let percent = (jaccard * 100.0).round().clamp(0.0, 100.0) as u32;
    percent
}

#[cfg(test)]
mod tests {
    use super::{
        outcome_of, score, sentence, Frequencies, Outcome, Side, SideFile, TextFacts,
        COMMON_VALUE_MAX,
    };
    use tam_types::ContentHash;

    fn digest(seed: u8) -> ContentHash {
        ContentHash([seed; 32])
    }

    fn file(seed: u8, byte_len: u64) -> SideFile {
        SideFile {
            digest: digest(seed),
            byte_len,
            name: Some("worksheet-pack.pdf".to_owned()),
        }
    }

    fn sketch(fill: u32, chars: u32) -> TextFacts {
        TextFacts {
            simhash: 0,
            minhash: [fill; 128],
            extracted_chars: chars,
        }
    }

    #[test]
    fn an_exact_large_rare_digest_merges_without_asking() {
        let side = Side {
            title: "Fractions pack".to_owned(),
            title_norm: "fractions pack".to_owned(),
            files: vec![file(7, 2_500_000)],
            ..Side::default()
        };
        let scored = score(&side, &side.clone(), &Frequencies::default(), 0.0);
        assert_eq!(outcome_of(&scored), Outcome::Same);
        assert_eq!(
            sentence(&scored),
            "The same file, byte for byte: worksheet-pack.pdf, 2.4 MB."
        );
    }

    #[test]
    fn a_digest_on_three_products_earns_nothing() {
        let side = Side {
            title_norm: "terms of use".to_owned(),
            files: vec![file(9, 2_500_000)],
            ..Side::default()
        };
        let mut frequency = Frequencies::default();
        frequency.digests.insert(digest(9).0, COMMON_VALUE_MAX + 1);
        let scored = score(&side, &side.clone(), &frequency, 0.0);
        assert_eq!(
            outcome_of(&scored),
            Outcome::Different,
            "a file every product carries is the seller's boilerplate, not an identity"
        );
    }

    #[test]
    fn a_small_exact_file_is_asked_rather_than_merged() {
        let side = Side {
            title: "Licence".to_owned(),
            title_norm: "licence".to_owned(),
            files: vec![file(3, 1_024)],
            ..Side::default()
        };
        let scored = score(&side, &side.clone(), &Frequencies::default(), 0.0);
        assert!(
            !scored.decisive,
            "a one-kilobyte file is as likely to be a licence note as a resource, so it never \
             merges unattended"
        );
        assert_eq!(outcome_of(&scored), Outcome::Ask);
    }

    #[test]
    fn strong_text_with_an_equal_page_count_merges_and_without_one_asks() {
        let with_pages = Side {
            title_norm: "long division".to_owned(),
            text: Some(sketch(11, 4_000)),
            page_count: Some(12),
            ..Side::default()
        };
        let merged = score(
            &with_pages,
            &with_pages.clone(),
            &Frequencies::default(),
            0.0,
        );
        assert_eq!(outcome_of(&merged), Outcome::Same);

        let no_pages = Side {
            page_count: None,
            ..with_pages.clone()
        };
        let asked = score(&no_pages, &no_pages.clone(), &Frequencies::default(), 0.0);
        assert_eq!(
            outcome_of(&asked),
            Outcome::Ask,
            "text alone is strong and not decisive; the page count is the conjunct"
        );
    }

    #[test]
    fn a_page_count_far_apart_refuses_the_merge_it_would_otherwise_earn() {
        let lhs = Side {
            title_norm: "long division".to_owned(),
            text: Some(sketch(11, 4_000)),
            page_count: Some(12),
            files: vec![file(7, 2_500_000)],
            ..Side::default()
        };
        let rhs = Side {
            page_count: Some(40),
            ..lhs.clone()
        };
        let scored = score(&lhs, &rhs, &Frequencies::default(), 0.0);
        assert!(scored.negative);
        assert_ne!(
            outcome_of(&scored),
            Outcome::Same,
            "no positive evidence overrides the one strong negative for an unattended merge"
        );
    }

    #[test]
    fn moderate_text_and_a_near_title_reach_the_review_floor() {
        let lhs = Side {
            title: "Fractions worksheet pack".to_owned(),
            title_norm: "fractions worksheet pack".to_owned(),
            text: Some(sketch(21, 3_000)),
            ..Side::default()
        };
        let mut rhs = lhs.clone();
        // Half the sketch agrees, which the estimator reads as a Jaccard of
        // about a half -- the moderate band.
        let mut minhash = [21_u32; 128];
        for slot in minhash.iter_mut().take(45) {
            *slot = 99;
        }
        rhs.text = Some(TextFacts {
            simhash: 0,
            minhash,
            extracted_chars: 3_000,
        });
        let scored = score(&lhs, &rhs, &Frequencies::default(), 0.0);
        assert_eq!(outcome_of(&scored), Outcome::Ask);
        assert_eq!(sentence(&scored), "The text of both PDFs is 65% the same.");
    }

    #[test]
    fn a_title_alone_is_never_enough() {
        let side = Side {
            title: "Fractions worksheet pack".to_owned(),
            title_norm: "fractions worksheet pack".to_owned(),
            ..Side::default()
        };
        let scored = score(&side, &side.clone(), &Frequencies::default(), 1.0);
        assert_eq!(
            outcome_of(&scored),
            Outcome::Different,
            "two listings can share a title and be different resources"
        );
    }
}
