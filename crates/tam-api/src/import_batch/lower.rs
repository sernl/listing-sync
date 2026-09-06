//! One claimed spreadsheet row, lowered into the create body the console's own
//! form sends.
//!
//! Nothing here is a second authoring path. The answer is a
//! [`CreateProductBody`] and [`crate::catalogue::create_one`] is what acts on
//! it, so every rule the create form meets — the title cap, the price
//! constructor, the required-field check, the held-bytes and picture-slot
//! checks, the listing quota — the import meets by taking the same road. What
//! this module decides is only which of the sheet's cells becomes which of
//! that body's fields.
//!
//! Three decisions, and each of them is about a cell that would otherwise be
//! lost. A native bound to an equivalence axis becomes an already-answered
//! election, which is the shape the create route records for a seller who
//! answered on the form. A native bound to two axes becomes one election per
//! binding, because both readings are true of the value and neither is ours to
//! prefer. A native bound to no axis at all becomes residue, kept verbatim,
//! because the alternative is dropping a cell a clean report accepted.

use std::collections::BTreeMap;

use tam_domain::registry::registry;
use tam_domain::TermKind;
use tam_storage::{ClaimedRow, RowFile};
use tam_types::{CopyFormat, ImportedTerm, InventoryId};

use crate::catalogue::{
    hex_encode, AnswerInput, CreateProductBody, ElectionInput, FileHandle, RightsInput,
};
use crate::error::APIError;

use super::report::RowDraft;
use super::validation;

/// The election trigger a cell the seller typed answers under.
///
/// `elect_one` is the one kind of the four that carries no trigger key: a
/// `supply` keys off the price and a `narrow` off the source value that was
/// narrowed, and a sheet cell is neither — it is an answer given before any
/// mapping exists, which is what the create form's own pre-answered elections
/// are.
const PRE_ANSWERED: &str = "elect_one";

/// One row's create, and the labels that follow it.
///
/// The labels travel beside the body rather than in it because they are a
/// second call: `product_label` names a product, so it cannot be written until
/// the product exists.
pub(crate) struct Lowered {
    pub body: CreateProductBody,
    pub labels: Vec<String>,
}

/// Builds the create one claimed row asks for.
pub(crate) fn lower(row: &ClaimedRow) -> Result<Lowered, APIError> {
    let draft: RowDraft = serde_json::from_value(row.draft.clone())
        .map_err(|_| validation("this row's parsed draft can no longer be read"))?;
    let RowDraft {
        title,
        body: copy,
        price,
        labels,
        // The row's own column is read instead; see below.
        inventories: _,
        natives,
        file_name,
    } = draft;

    let payload = row
        .file
        .as_ref()
        .map(|file| handle_of(file, file_name))
        .transpose()?;
    // No name for the cover: it is generated from the payload's own bytes
    // during the upload rather than chosen, which is the position the create
    // route already takes on it.
    let cover = row
        .cover
        .as_ref()
        .map(|file| handle_of(file, None))
        .transpose()?;

    let mut body = CreateProductBody {
        title,
        body: copy,
        // The create form's own default. The sheet has one Description column
        // and no column to state a format in, so the import cannot claim one
        // the seller did not choose.
        body_format: CopyFormat::Markdown,
        price,
        payload: payload.into_iter().collect(),
        cover,
        previews: Vec::new(),
        subjects: Vec::new(),
        grades: Vec::new(),
        rights: None,
        // The column rather than the draft's copy of it. The claim reserved a
        // mapping identifier exactly where this column is set, so reading the
        // same column here is what keeps the mapping count and the inventory
        // count from disagreeing.
        inventories: row.inventory.into_iter().collect(),
        elections: Vec::new(),
        // The sheet writes no TPT-base block: those controls are TPT's own
        // form and the template carries no column for them.
        tpt_base: None,
        natives: Vec::new(),
    };

    if let Some(inventory) = row.inventory {
        place_natives(&mut body, inventory, &natives);
    }

    Ok(Lowered { body, labels })
}

/// Routes every native cell the seller filled to the field that holds it.
///
/// Three destinations and no fourth, which is what makes the routing total: a
/// licence is a rights grant, another bound axis is an election, and an
/// unbound native is residue. A Teachouse row reaches none of them, because it
/// names no marketplace and therefore has no registry to read a binding out of.
fn place_natives(
    body: &mut CreateProductBody,
    inventory: InventoryId,
    natives: &BTreeMap<String, Vec<String>>,
) {
    let axes = registry(inventory).equivalence_axes;
    for (native, values) in natives {
        let bound: Vec<TermKind> = axes
            .iter()
            .filter(|binding| binding.native == native.as_str())
            .map(|binding| binding.axis)
            .collect();
        if bound.is_empty() {
            body.natives.extend(values.iter().map(|value| ImportedTerm {
                inventory,
                // Kind-less deliberately: which axis an unbound native answers
                // is a fact of a relation nobody has seeded, and labelling it
                // with an existing kind would project the value rather than
                // keep it.
                kind: None,
                segments: vec![value.clone()],
                native_id: Some(value.clone()),
            }));
            continue;
        }
        for axis in bound {
            match axis {
                // A licence is the product's own rights declaration rather than
                // an election, which is one of the two shapes
                // `required_fields_answered` accepts. One value, because the
                // column is single-valued: `Cardinality::One` makes the parse
                // read the cell whole rather than split it.
                TermKind::Licence => {
                    body.rights = values.first().map(|value| RightsInput {
                        inventory,
                        segments: vec![value.clone()],
                        native_id: Some(value.clone()),
                    });
                }
                TermKind::Subject | TermKind::Topic | TermKind::ResourceType | TermKind::Phase => {
                    body.elections.push(ElectionInput {
                        inventory,
                        axis,
                        trigger: PRE_ANSWERED.to_owned(),
                        trigger_key: None,
                        answers: values
                            .iter()
                            .map(|value| AnswerInput {
                                segments: vec![value.clone()],
                                native_id: Some(value.clone()),
                            })
                            .collect(),
                    });
                }
            }
        }
    }
}

/// One stored handle as the create route takes it.
///
/// The kind is the column's, which the bind already wrote in the vocabulary's
/// own spelling, so nothing is re-derived here from a caller's word.
fn handle_of(file: &RowFile, name: Option<String>) -> Result<FileHandle, APIError> {
    let byte_len = u64::try_from(file.byte_len).map_err(|_| {
        validation("this row's file handle states a length no stored file can have")
    })?;
    Ok(FileHandle {
        hash: hex_encode(&file.hash.0),
        kind: file.kind.clone(),
        byte_len,
        name,
    })
}
