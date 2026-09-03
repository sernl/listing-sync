//! The education-standards half of the TPT adapter: the two GraphQL
//! operations that enumerate TPT's own standards tree, and the mapping from a
//! resolved node id onto the create form's wire fields.
//!
//! TPT binds a standard by an opaque numeric node id rather than by the
//! published code, so nothing here can be composed from a code alone. An id
//! comes from the tree, and the tree is reachable only by walking it:
//! `EducationStandardsJurisdictionsQuery` returns the 166 roots and
//! `EducationStandardsQuery` expands one node at a time, `depth: 1` for a
//! jurisdiction's top level and no depth argument below it, which is how
//! TPT's own picker drives them across the capture's 58 calls
//! (`docs/research/rethink/tpt-product-model.md:340`).
//!
//! Neither operation appears in a HAR capture. Their selection sets are
//! recorded in that research note and in `docs/design/data/tpt-vocabulary.json`,
//! and the query texts below are reconstructed from those records rather than
//! lifted from a recording, which is the one place this module rests on
//! weaker evidence than the rest of the crate. The two `children` argument
//! names are inferred from the recorded variable signature; the founder's
//! first capture is what confirms them.
//!
//! This module composes requests and parses answers and issues nothing. Who
//! runs the crawl is settled elsewhere and is not its business: the founder,
//! on the founder's own machine under the founder's own session, through
//! `tam-standards-crawl`.
//!
//! The posting half is deliberately small. A caller holding a node id the
//! current crawl window vouches for gets the wire fields; a caller holding
//! anything else gets a record of what will not be carried, which is what the
//! seller reads in the field diff before publish. That is decision 6 of
//! `docs/notes/design/standards-ingestion.md`: while the table is empty a
//! seller may tag standards, and publish omits them visibly rather than
//! silently.

use serde_json::{json, Value};
use tam_marketplace::transport::HttpRequest;

use crate::endpoints::{Service, ORIGIN};
use crate::read_model::ShapeError;

/// TPT's own numeric id for a node of its education-standards tree: what
/// `EducationStandardsQuery` returns as `id`, what the product read aliases
/// as `sphinxId`, and what a `common_core_standard_id[]` part carries.
///
/// The catalogue side carries the same value as `tam_standards::TptNodeId`.
/// It is declared twice rather than shared because this adapter holds no
/// dependency edge to the catalogue crate: an adapter that could read the
/// standards catalogue is an adapter that could resolve a code into an id
/// itself, and that resolution belongs to the crawl.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StandardNodeId(pub u64);

impl core::fmt::Display for StandardNodeId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub const JURISDICTIONS_OPERATION: &str = "EducationStandardsJurisdictionsQuery";
pub const SUBTREE_OPERATION: &str = "EducationStandardsQuery";

/// The enumeration. There is no public listing of TPT's supported standards,
/// so this operation is the enumeration rather than a convenience over one.
pub const JURISDICTIONS_QUERY: &str = r"query EducationStandardsJurisdictionsQuery {
  educationStandards {
    jurisdictions: roots {
      id
      name
      notation
    }
  }
}
";

/// The top-level expansion, which is the only call the recording makes with a
/// depth: `depth: 1` returns a jurisdiction's own children, `depth: 2` those
/// plus theirs, so the argument counts levels and the answer is flat.
pub const SUBTREE_QUERY_AT_DEPTH: &str = r"query EducationStandardsQuery($id: ID!, $depth: Int) {
  educationStandards {
    children(id: $id, depth: $depth) {
      id
      name
      depth
      notation
      grades
      type
      parentIds
      sequence
      categorySequence
      descriptionText
      descriptionHtml
    }
  }
}
";

/// The deeper expansion. The picker sends no depth argument at all below a
/// jurisdiction's top level, so this is a second query text rather than the
/// one above with a null variable: the recorded distinction is between two
/// call shapes, and collapsing them would be a guess about what the server
/// does with a null depth.
pub const SUBTREE_QUERY: &str = r"query EducationStandardsQuery($id: ID!) {
  educationStandards {
    children(id: $id) {
      id
      name
      depth
      notation
      grades
      type
      parentIds
      sequence
      categorySequence
      descriptionText
      descriptionHtml
    }
  }
}
";

fn url(operation: &str) -> String {
    format!("{ORIGIN}{}?opname={operation}", Service::Graph.path())
}

fn graphql(operation: &str, query: &str, variables: &Value) -> HttpRequest {
    HttpRequest::post_json(
        url(operation),
        json!({
            "operationName": operation,
            "variables": variables,
            "extensions": {},
            "query": query,
        }),
    )
}

#[must_use]
pub fn jurisdictions_request() -> HttpRequest {
    graphql(JURISDICTIONS_OPERATION, JURISDICTIONS_QUERY, &json!({}))
}

/// Expand one node. `Some(1)` at a jurisdiction and `None` below it is the
/// pattern the capture recorded; any other depth is the caller's to justify.
///
/// The id travels as a string because the variable is declared `ID!`, whose
/// input coercion is defined for a string on every server.
#[must_use]
pub fn subtree_request(node: StandardNodeId, depth: Option<u32>) -> HttpRequest {
    match depth {
        Some(levels) => graphql(
            SUBTREE_OPERATION,
            SUBTREE_QUERY_AT_DEPTH,
            &json!({ "id": node.to_string(), "depth": levels }),
        ),
        None => graphql(
            SUBTREE_OPERATION,
            SUBTREE_QUERY,
            &json!({ "id": node.to_string() }),
        ),
    }
}

/// One of the roots the jurisdictions query answers with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JurisdictionRoot {
    pub id: StandardNodeId,
    pub name: String,
    pub notation: Option<String>,
}

/// One node of TPT's standards tree, as the expansion returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub id: StandardNodeId,
    /// The node's own name, which at a leaf is the published standard code:
    /// `CCRA.L.1` rather than the mirror's `CCSS.ELA-Literacy.CCRA.L.1`.
    pub name: String,
    pub depth: u32,
    /// The tree level's label. The capture holds `standard`, `domain`,
    /// `cluster` and `subject`; only `standard` is a node the form posts, and
    /// the rest are the tree a crawl walks through.
    pub node_type: Option<String>,
    /// The full ancestor chain rather than a single parent.
    pub parent_ids: Vec<StandardNodeId>,
    /// `descriptionText` and never `descriptionHtml`: a statement is carried
    /// verbatim or not at all, and the hash a binding records is over these
    /// bytes.
    pub statement: Option<String>,
}

/// TPT serialises an id as a number in one place and a string in another, so
/// an id is read through this rather than through one of the two accessors.
fn node_id(value: &Value) -> Option<StandardNodeId> {
    match value {
        Value::Number(number) => number.as_u64().map(StandardNodeId),
        Value::String(text) => text.parse().ok().map(StandardNodeId),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

fn rows<'a>(body: &'a Value, pointer: &str) -> Result<&'a [Value], ShapeError> {
    body.pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| ShapeError(format!("no {pointer} array in the response")))
}

fn required_str(row: &Value, field: &str, id: StandardNodeId) -> Result<String, ShapeError> {
    row.get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ShapeError(format!("standards node {id} carries no {field}")))
}

fn optional_str(row: &Value, field: &str) -> Option<String> {
    row.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn read_id(row: &Value) -> Result<StandardNodeId, ShapeError> {
    row.get("id")
        .and_then(node_id)
        .ok_or_else(|| ShapeError("a standards node carries no numeric id".to_owned()))
}

/// Parses the enumeration. A root that does not parse fails the whole answer
/// rather than being dropped: a crawl that silently skipped a root would
/// write a table short by a framework and say nothing.
pub fn parse_jurisdictions(body: &Value) -> Result<Vec<JurisdictionRoot>, ShapeError> {
    rows(body, "/data/educationStandards/jurisdictions")?
        .iter()
        .map(|row| {
            let id = read_id(row)?;
            Ok(JurisdictionRoot {
                id,
                name: required_str(row, "name", id)?,
                notation: optional_str(row, "notation"),
            })
        })
        .collect()
}

/// Parses one expansion. A node without an id or a name fails the answer for
/// the reason above: a dropped node is a standard that silently cannot be
/// tagged.
pub fn parse_subtree(body: &Value) -> Result<Vec<TreeNode>, ShapeError> {
    rows(body, "/data/educationStandards/children")?
        .iter()
        .map(|row| {
            let id = read_id(row)?;
            let parent_ids = row
                .get("parentIds")
                .and_then(Value::as_array)
                .map(|parents| parents.iter().filter_map(node_id).collect())
                .unwrap_or_default();
            Ok(TreeNode {
                id,
                name: required_str(row, "name", id)?,
                depth: row
                    .get("depth")
                    .and_then(Value::as_u64)
                    .and_then(|depth| u32::try_from(depth).ok())
                    .unwrap_or_default(),
                node_type: optional_str(row, "type"),
                parent_ids,
                statement: optional_str(row, "descriptionText"),
            })
        })
        .collect()
}

/// The tree level the form posts. Every other level is structure.
pub const BINDABLE_NODE_TYPE: &str = "standard";

/// The two wire names the standards block emits, held here because
/// `write_model`'s own name table is private to that module. Both are
/// attested by the `data[_Token][unlocked]` list the committed create and
/// edit form cassettes carry.
pub const STANDARDS_COUNT_FIELD: &str = "data[ItemsCommonCoreStandard][common_core_standards_num]";
pub const STANDARD_ID_FIELD: &str = "data[ItemsCommonCoreStandard][common_core_standard_id][]";

/// Why a standard the seller tagged is not carried to TPT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unpostable {
    /// No crawl has bound this standard to a node id. The whole table today,
    /// and the case decision 6 answers: the tag is kept in our own catalogue
    /// and the publish omits it visibly.
    Unbound,
    /// A binding exists and the current crawl window does not vouch for it.
    /// `sphinxId` names a search index and search indexes get rebuilt, so an
    /// unvouched id may now denote a different standard; posting it would tag
    /// the wrong one silently, which is worse than posting nothing.
    OutsideCrawlWindow,
}

impl Unpostable {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unbound => "unbound",
            Self::OutsideCrawlWindow => "outside_crawl_window",
        }
    }
}

/// What the caller resolved for one tagged standard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    Postable(StandardNodeId),
    Unpostable(Unpostable),
}

/// One standard the seller tagged, with the resolution the caller reached for
/// it. The resolution happens on the catalogue side, where the node-id table
/// and the crawl window live; this module only turns it into wire fields or
/// into a record of what will not be carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedStandard {
    /// The catalogue's own identifier for the standard, carried so a record
    /// names the row the seller picked. A published code cannot do that job:
    /// 697 of 4,872 Texas codes name a different standard under a different
    /// subject.
    pub source_guid: String,
    /// The published code, for the seller-facing record rather than for the
    /// join.
    pub code: String,
    pub binding: Binding,
}

/// One standard the publish will not carry, named so the field diff can show
/// it before the write rather than after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotCarried {
    pub source_guid: String,
    pub code: String,
    pub reason: Unpostable,
}

/// The standards block of a create or edit body, and what it left behind.
///
/// The two field groups are separate because the form posts them apart: the
/// count travels early, among the thumbnail fields, and the id parts late,
/// after the categories. A caller splices each at its own recorded position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardsProjection {
    /// `common_core_standards_num`, which the captures record as equal to the
    /// number of id parts.
    pub count_field: (String, String),
    /// One part per bound standard, or the single empty part a body with no
    /// standards posts.
    pub id_fields: Vec<(String, String)>,
    /// Never blocks a publish. A loss is disclosed, not decided.
    pub not_carried: Vec<NotCarried>,
}

fn count_field(bound: &[StandardNodeId]) -> (String, String) {
    (STANDARDS_COUNT_FIELD.to_owned(), bound.len().to_string())
}

fn id_fields(bound: &[StandardNodeId]) -> Vec<(String, String)> {
    if bound.is_empty() {
        return vec![(STANDARD_ID_FIELD.to_owned(), String::new())];
    }
    bound
        .iter()
        .map(|id| (STANDARD_ID_FIELD.to_owned(), id.to_string()))
        .collect()
}

/// Turns what the caller resolved into the body's standards block.
///
/// A node bound twice is posted once, because two catalogue rows can name one
/// TPT node — the mirror repeats an anchor standard across grade sets — and
/// the count the form expects is the number of id parts.
#[must_use]
pub fn project_standards(selected: &[SelectedStandard]) -> StandardsProjection {
    let mut bound: Vec<StandardNodeId> = Vec::new();
    let mut not_carried = Vec::new();
    for standard in selected {
        match standard.binding {
            Binding::Postable(id) => {
                if !bound.contains(&id) {
                    bound.push(id);
                }
            }
            Binding::Unpostable(reason) => not_carried.push(NotCarried {
                source_guid: standard.source_guid.clone(),
                code: standard.code.clone(),
                reason,
            }),
        }
    }
    StandardsProjection {
        count_field: count_field(&bound),
        id_fields: id_fields(&bound),
        not_carried,
    }
}

#[cfg(test)]
mod tests {
    use tam_marketplace::transport::{Method, RequestBody};

    use super::*;

    fn sent(request: &HttpRequest) -> Value {
        let RequestBody::Json(value) = &request.body else {
            panic!("both operations post JSON");
        };
        value.clone()
    }

    /// One jurisdiction expansion, in the shape the research records: a flat
    /// list carrying the levels above the leaves as well as the leaves.
    const SUBTREE: &str = r#"{"data":{"educationStandards":{"children":[
      {"id":3052,"name":"English Language Arts","depth":1,"type":"subject","parentIds":[3054],"descriptionText":null},
      {"id":"9001","name":"CCRA.L.1","depth":4,"type":"standard","parentIds":[3054,3052,8001],
       "descriptionText":"Demonstrate command of the conventions of standard English grammar and usage when writing or speaking."}
    ]}}}"#;

    #[test]
    fn the_enumeration_addresses_the_graph_service_by_operation_name() {
        let request = jurisdictions_request();
        assert_eq!(request.method, Method::Post, "both operations are posts");
        assert_eq!(
            request.url,
            "https://www.teacherspayteachers.com/graph/graphql?opname=EducationStandardsJurisdictionsQuery",
            "the client addresses each operation by name in the query string"
        );
        let body = sent(&request);
        assert_eq!(body["operationName"], json!(JURISDICTIONS_OPERATION));
        assert_eq!(
            body["query"],
            json!(JURISDICTIONS_QUERY),
            "the full query text travels: TPT uses no persisted queries"
        );
    }

    #[test]
    fn a_top_level_expansion_carries_a_depth_and_a_deeper_one_carries_none() {
        let top = sent(&subtree_request(StandardNodeId(3054), Some(1)));
        assert_eq!(
            top["variables"],
            json!({"id": "3054", "depth": 1}),
            "the jurisdiction's own children, one level"
        );
        assert_eq!(top["query"], json!(SUBTREE_QUERY_AT_DEPTH));

        let deeper = sent(&subtree_request(StandardNodeId(3052), None));
        assert_eq!(
            deeper["variables"],
            json!({ "id": "3052" }),
            "no depth argument at all below the top level, as the picker sends it"
        );
        assert_eq!(deeper["query"], json!(SUBTREE_QUERY));
        assert_eq!(
            deeper["operationName"], top["operationName"],
            "one operation name covers both call shapes"
        );
    }

    #[test]
    fn roots_parse_under_either_id_encoding() {
        let body: Value = serde_json::from_str(
            r#"{"data":{"educationStandards":{"jurisdictions":[
              {"id":3054,"name":"Common Core State Standards","notation":"ccss"},
              {"id":"8044","name":"Ontario Curriculum","notation":null}
            ]}}}"#,
        )
        .expect("fixture");
        let roots = parse_jurisdictions(&body).expect("both roots parse");
        assert_eq!(
            roots,
            vec![
                JurisdictionRoot {
                    id: StandardNodeId(3054),
                    name: "Common Core State Standards".to_owned(),
                    notation: Some("ccss".to_owned()),
                },
                JurisdictionRoot {
                    id: StandardNodeId(8044),
                    name: "Ontario Curriculum".to_owned(),
                    notation: None,
                },
            ],
            "138 of the 166 roots carry no notation and are still roots"
        );
    }

    #[test]
    fn a_root_without_an_id_fails_the_answer_rather_than_being_dropped() {
        let body: Value = serde_json::from_str(
            r#"{"data":{"educationStandards":{"jurisdictions":[{"name":"nameless"}]}}}"#,
        )
        .expect("fixture");
        parse_jurisdictions(&body).expect_err("a crawl must not silently shorten its enumeration");
    }

    #[test]
    fn an_answer_that_is_not_an_expansion_is_refused() {
        let body: Value =
            serde_json::from_str(r#"{"data":{"educationStandards":{}}}"#).expect("fixture");
        parse_subtree(&body).expect_err("no children array is not an empty subtree");
    }

    #[test]
    fn an_expansion_carries_the_code_the_level_and_the_statement() {
        let body: Value = serde_json::from_str(SUBTREE).expect("fixture");
        let nodes = parse_subtree(&body).expect("both nodes parse");
        assert_eq!(nodes.len(), 2, "the answer is flat and carries both levels");

        let subject = &nodes[0];
        assert_eq!(subject.node_type.as_deref(), Some("subject"));
        assert_eq!(
            subject.statement, None,
            "a level above a leaf carries no statement"
        );

        let leaf = &nodes[1];
        assert_eq!(leaf.id, StandardNodeId(9001), "the id the form would post");
        assert_eq!(
            leaf.name, "CCRA.L.1",
            "TPT's name is the published code, without the mirror's namespace prefix"
        );
        assert_eq!(leaf.node_type.as_deref(), Some(BINDABLE_NODE_TYPE));
        assert_eq!(
            leaf.parent_ids,
            vec![
                StandardNodeId(3054),
                StandardNodeId(3052),
                StandardNodeId(8001)
            ],
            "parentIds is the whole ancestor chain rather than one parent"
        );
        assert!(
            leaf.statement
                .as_deref()
                .is_some_and(|statement| statement.starts_with("Demonstrate command")),
            "the statement travels verbatim, got {:?}",
            leaf.statement
        );
    }

    #[test]
    fn a_listing_with_nothing_bound_posts_the_empty_shape_and_the_loss_record() {
        let selected = vec![
            SelectedStandard {
                source_guid: "A6F2".to_owned(),
                code: "CCSS.Math.Content.8.F.B.5".to_owned(),
                binding: Binding::Unpostable(Unpostable::Unbound),
            },
            SelectedStandard {
                source_guid: "M1".to_owned(),
                code: "1.1.A".to_owned(),
                binding: Binding::Unpostable(Unpostable::OutsideCrawlWindow),
            },
        ];
        let projection = project_standards(&selected);
        assert_eq!(
            projection.count_field,
            (STANDARDS_COUNT_FIELD.to_owned(), "0".to_owned()),
            "the count equals the number of id parts, which is none"
        );
        assert_eq!(
            projection.id_fields,
            vec![(STANDARD_ID_FIELD.to_owned(), String::new())],
            "the name is still posted, with the empty value a body carrying no standards posts"
        );
        assert_eq!(
            projection
                .not_carried
                .iter()
                .map(|record| (record.code.as_str(), record.reason))
                .collect::<Vec<_>>(),
            vec![
                ("CCSS.Math.Content.8.F.B.5", Unpostable::Unbound),
                ("1.1.A", Unpostable::OutsideCrawlWindow),
            ],
            "both are disclosed, each with the reason the seller is owed"
        );
    }

    #[test]
    fn bound_standards_post_one_part_each_under_a_count_that_equals_them() {
        let selected = vec![
            SelectedStandard {
                source_guid: "A6F2".to_owned(),
                code: "CCSS.Math.Content.8.F.B.5".to_owned(),
                binding: Binding::Postable(StandardNodeId(9001)),
            },
            SelectedStandard {
                source_guid: "B7E3".to_owned(),
                code: "CCSS.ELA-Literacy.CCRA.L.1".to_owned(),
                binding: Binding::Postable(StandardNodeId(9002)),
            },
        ];
        let projection = project_standards(&selected);
        assert_eq!(
            projection.count_field,
            (STANDARDS_COUNT_FIELD.to_owned(), "2".to_owned())
        );
        assert_eq!(
            projection.id_fields,
            vec![
                (STANDARD_ID_FIELD.to_owned(), "9001".to_owned()),
                (STANDARD_ID_FIELD.to_owned(), "9002".to_owned()),
            ],
            "one repeated part per node, in the order the seller's selection carried"
        );
        assert!(
            projection.not_carried.is_empty(),
            "nothing was left behind, got {:?}",
            projection.not_carried
        );
    }

    #[test]
    fn one_node_reached_by_two_catalogue_rows_is_posted_once_and_counted_once() {
        let selected = vec![
            SelectedStandard {
                source_guid: "grade-4-set".to_owned(),
                code: "CCSS.ELA-Literacy.CCRA.L.1".to_owned(),
                binding: Binding::Postable(StandardNodeId(9002)),
            },
            SelectedStandard {
                source_guid: "grade-5-set".to_owned(),
                code: "CCSS.ELA-Literacy.CCRA.L.1".to_owned(),
                binding: Binding::Postable(StandardNodeId(9002)),
            },
        ];
        let projection = project_standards(&selected);
        assert_eq!(
            projection.id_fields.len(),
            1,
            "the mirror repeats an anchor standard across grade sets; TPT holds one node"
        );
        assert_eq!(
            projection.count_field.1, "1",
            "the count is the number of parts, not the number of rows the seller picked"
        );
    }
}
