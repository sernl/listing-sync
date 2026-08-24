# Probe: Tes uploader vocabulary

- date: 2026-08-25
- account: EBMC (International market)
- method: DOM option-set inspection across the wizard, plus a public taxonomy API probe
- evidence: classification and files screenshots; probes/local/tax-*.json; docs/design/data/tes-vocabulary.json

## Observation

The uploader vocabulary is a set of numeric ids with human labels, and the subject and topic tree is served by a public API.
`GET /taxonomy/v4/{country}/{id}` returns unauthenticated JSON carrying `id`, `country`, `parentId`, `path` and a `leaf` depth marker, so the entire subject and topic tree is crawlable by the product with no session, keyed by a country code such as GB.
The resource-type and age-range vocabularies are fixed option sets read directly from the form.

Resource type, the `mainType` field, is 99001 Assembly, 99002 Assessment and revision, 99003 Game/puzzle/quiz, 99004 Audio, music and video, 99005 Lesson (complete), 99006 Other, 99007 Unit of work, 99008 Visual aid/Display, 99009 Worksheet/Activity.
Main age range is 1 for 3-5, 2 for 5-7, 3 for 7-11, 4 for 11-14, 5 for 14-16, 6 for 16+, and 7 for age not applicable, and these boundaries are the grade-model data the design requires and forbids transcribing from memory.
Curriculum is None, No curriculum, American, Australian, Canadian, English, International, Irish, New Zealand, Northern Irish, Scottish, Welsh and Zambian.
Subjects and topics are numeric ids, for example Mathematics topics 1000448 Algebra, 1000897 Number, 1000903 Geometry and measures, 1000977 Data and statistics, and the categories field caps at roughly ten subject-and-topic pairs.

## Answer to the gated question

The taxonomy work is bounded and largely agent-automatable, because the subject and topic tree is a public crawlable API and the fixed option sets are small.
The full per-country vocabulary crawl needs no founder session and is a clean follow-up task.

## Confidence

High for the resource-type, age-range and curriculum sets, which are read directly from the DOM.
High that the subject and topic tree is publicly crawlable, which is directly probed; medium on total size until the crawl runs.
