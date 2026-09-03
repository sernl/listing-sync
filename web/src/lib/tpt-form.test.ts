import { describe, expect, it } from 'vitest';
import { loadCoreForTest } from '$lib/core/testing';
import {
	applyToAll,
	atCap,
	capOf,
	counterOf,
	createBodyOf,
	diverges,
	divergentOn,
	draftInputOf,
	emptyTptDraft,
	gradeColumns,
	labelOf,
	minorUnitsOf,
	OVERRIDABLE,
	projectionOf,
	refusalsOf,
	refusalsIn,
	searchFacets,
	submittable,
	suggestedAdditionalLicence,
	togglePick,
	tptBaseOf,
	valueFor,
	withOverride,
	type MarketplaceProjection,
	type TptDraft
} from './tpt-form';
import type { FacetView, FormVocabularyView, VocabularyView } from '$lib/api';

// The refusals below are the compiled core's, not this file's, so the module
// has to be in memory before any of them is asked for.
loadCoreForTest();

/** The vocabulary as `GET /v1/authoring/vocabulary` serves it, trimmed to the
 *  fields these tests read. Transcribed from `crates/tam-api/src/product/mod.rs`
 *  and the committed capture rather than invented; the caps are the numbers
 *  `TptForm` reads, including the absent subject-area one. */
function facet(slug: string, label: string, writable = true): FacetView {
	return { slug, label, seller_writable: writable };
}

const VOCABULARY: FormVocabularyView = {
	grades: [
		facet('preschool', 'Preschool'),
		facet('kindergarten', 'Kindergarten'),
		facet('1st-grade', '1st Grade'),
		facet('2nd-grade', '2nd Grade'),
		facet('3rd-grade', '3rd Grade'),
		facet('4th-grade', '4th Grade'),
		facet('5th-grade', '5th Grade'),
		facet('6th-grade', '6th Grade'),
		facet('7th-grade', '7th Grade'),
		facet('8th-grade', '8th Grade'),
		facet('9th-grade', '9th Grade'),
		facet('10th-grade', '10th Grade'),
		facet('11th-grade', '11th Grade'),
		facet('12th-grade', '12th Grade'),
		facet('higher-education', 'Higher Education'),
		facet('adult-education', 'Adult Education'),
		facet('not-grade-specific', 'Not Grade Specific'),
		facet('elementary', 'Elementary', false),
		facet('middle-school', 'Middle School', false),
		facet('high-school', 'High School', false)
	],
	grade_columns: [5, 5, 4, 3],
	subject_areas: [facet('math', 'Math'), facet('science', 'Science'), facet('art', 'Art')],
	tags: [facet('centers', 'Centers'), facet('autumn', 'Autumn')],
	formats: [facet('easel', 'Easel')],
	tax_codes: [
		{
			id: '1',
			label: 'Digital audio works sold to an end user with rights for permanent use',
			code: 'DA051011'
		},
		{ id: '2', label: 'Digital books sold to an end user with rights for permanent use', code: 'DB031013' }
	],
	teaching_durations: [
		{ id: '0', label: 'N/A' },
		{ id: '1', label: '30 Minutes' }
	],
	answer_keys: [
		{ id: '0', label: 'N/A', menu_index: 0 },
		{ id: '1', label: 'Included', menu_index: 1 },
		{ id: '2', label: 'Not Included', menu_index: 2 },
		{ id: '3', label: 'Does Not Apply', menu_index: 5 },
		{ id: '4', label: 'Included with Rubric', menu_index: 3 },
		{ id: '5', label: 'Rubric Only', menu_index: 4 }
	],
	thumbnail_modes: [
		{ id: '1', label: 'Auto generate thumbnails from the product file' },
		{ id: '2', label: 'Upload thumbnails now' },
		{ id: '3', label: 'Upload thumbnails later' }
	],
	copyright: {
		preamble: 'Intellectual Property Rights: …',
		options: [
			{ id: '1', label: 'I attest that this product I am about to post is an original work…' },
			{ id: '2', label: 'I attest that I have used copyrighted and/or trademarked materials…' }
		],
		preselect: false
	},
	localisation: {
		label: null,
		generic_label: "Appropriate for your TPT account's country"
	},
	statuses: [
		{ id: '0', label: 'Draft, visible only to you' },
		{ id: '1', label: 'Make Listing Active' }
	],
	standards_frameworks: [
		{ jurisdiction_id: 3054, name: 'Common Core State Standards', button_label: 'Select CCSS' }
	],
	// The one cap a capture has contradicted is absent, which is what makes the
	// subject-area counter read as a plain count.
	caps: { grades: 4, tags: 6, formats: 3, thumbnails: 4 },
	limits: {
		title_max_utf16_units: 80,
		description_max_length: 45000,
		min_price_minor_units: 95,
		additional_licence_percentage: 90,
		free_resource_page_guidance: 10,
		product_file: { label: 'Downloadable File', max_size_bytes: 4294967296, file_extensions: [] },
		preview: { label: 'Preview', max_size_bytes: 31457280, file_extensions: [] },
		video_preview: { label: 'Video Preview', max_size_bytes: 1073741824, file_extensions: [] },
		thumbnail: { label: 'Thumbnail', max_size_bytes: 4194304, file_extensions: [] }
	}
};

/** A draft with every required control answered, so each test below changes
 *  one thing and the refusal names that one. */
function complete(): TptDraft {
	return {
		...emptyTptDraft(),
		name: 'Fractions on a number line',
		payload: [{ hash: 'a'.repeat(64), kind: 'pdf', byte_len: 1024 }],
		free: true,
		grades: ['3rd-grade'],
		subjectAreas: ['math'],
		tags: ['centers'],
		copyright: '1',
		inventories: ['Tpt']
	};
}

describe('the grade grid', () => {
	it('reads down four columns, which is the segregation TPT’s own grid shows', () => {
		const columns = gradeColumns(VOCABULARY);
		expect(columns.map((column) => column.length)).toEqual([5, 5, 4, 3]);
		expect(columns.map((column) => column[0].slug)).toEqual([
			'preschool',
			'4th-grade',
			'9th-grade',
			'higher-education'
		]);
	});

	it('omits the three roll-ups, which the create form renders no checkbox for', () => {
		const shown = gradeColumns(VOCABULARY).flat().map((grade) => grade.slug);
		expect(shown).not.toContain('elementary');
		expect(shown).toHaveLength(17);
	});
});

describe('the counters and the caps', () => {
	it('reads "n of cap" against a measured cap', () => {
		expect(counterOf(2, capOf(VOCABULARY.caps, 'grades'))).toEqual({ text: '2 of 4', over: false });
	});

	it('reads a plain count where the cap is unmeasured, rather than inventing a ceiling', () => {
		expect(capOf(VOCABULARY.caps, 'subjectAreas')).toBeNull();
		expect(counterOf(4, null)).toEqual({ text: '4 chosen', over: false });
	});

	it('stops at the cap instead of letting the seller discover it by refusal', () => {
		const four = ['preschool', 'kindergarten', '1st-grade', '2nd-grade'];
		expect(atCap(four.length, 4)).toBe(true);
		expect(togglePick(four, '3rd-grade', true, 4)).toEqual(four);
	});

	it('never stops an uncapped picker, because absent is unmeasured and not unlimited', () => {
		const three = ['math', 'science', 'art'];
		expect(atCap(three.length, null)).toBe(false);
		expect(togglePick(three, 'ela', true, null)).toHaveLength(4);
	});

	it('keeps the seller’s own order and ticks a held value only once', () => {
		expect(togglePick(['math'], 'math', true, null)).toEqual(['math']);
		expect(togglePick(['math', 'science'], 'math', false, null)).toEqual(['science']);
	});
});

describe('the pickers’ search', () => {
	it('matches the label or the slug, case-insensitively, and offers only writable facets', () => {
		expect(searchFacets(VOCABULARY.grades, 'grade').map((grade) => grade.slug)).toContain(
			'1st-grade'
		);
		expect(searchFacets(VOCABULARY.grades, '').map((grade) => grade.slug)).not.toContain(
			'elementary'
		);
		expect(searchFacets(VOCABULARY.subject_areas, 'MATH')).toHaveLength(1);
	});
});

describe('the copyright gate', () => {
	it('refuses submission until one of the two attestations is chosen', () => {
		const unstated = { ...complete(), copyright: null };
		const refusals = refusalsOf(unstated, VOCABULARY);
		expect(submittable(refusals)).toBe(false);
		expect(refusalsIn(refusals, 'copyright')).toHaveLength(1);
		expect(refusalsIn(refusals, 'copyright')[0].message).toContain('Nothing is pre-selected');
	});

	it('is not pre-selected on a blank draft, unlike TPT’s own form', () => {
		expect(emptyTptDraft().copyright).toBeNull();
		expect(VOCABULARY.copyright.preselect).toBe(false);
	});

	it('lets a complete draft through once it is stated', () => {
		expect(refusalsOf(complete(), VOCABULARY)).toEqual([]);
	});
});

describe('the refusals', () => {
	it('names the control a cap refusal is about', () => {
		const over = {
			...complete(),
			grades: ['preschool', 'kindergarten', '1st-grade', '2nd-grade', '3rd-grade']
		};
		const [refusal] = refusalsIn(refusalsOf(over, VOCABULARY), 'categories');
		expect(refusal.control).toBe('Grade Level');
		expect(refusal.message).toBe('Grade Level takes up to 4, and 5 are chosen.');
	});

	it('accepts four subject areas, because a create TPT accepted posted four', () => {
		const many = { ...complete(), subjectAreas: ['math', 'science', 'art', 'ela'] };
		expect(refusalsOf(many, VOCABULARY)).toEqual([]);
	});

	it('names each of the three required pickers separately', () => {
		const bare = { ...complete(), grades: [], subjectAreas: [], tags: [] };
		expect(refusalsIn(refusalsOf(bare, VOCABULARY), 'categories').map((r) => r.control)).toEqual([
			'Grade Level',
			'Subject Area',
			'Tag'
		]);
	});

	it('asks for a tax code on a paid listing and never chooses one', () => {
		const paid = { ...complete(), free: false, price: '4.50' };
		expect(refusalsIn(refusalsOf(paid, VOCABULARY), 'price').map((r) => r.control)).toEqual([
			'Tax Code'
		]);
		expect(emptyTptDraft().taxCode).toBeNull();
	});

	it('refuses a price under TPT’s own floor', () => {
		const cheap = { ...complete(), free: false, price: '0.50', taxCode: '1' };
		expect(refusalsIn(refusalsOf(cheap, VOCABULARY), 'price')[0].message).toContain('$0.95');
	});

	it('refuses thumbnails attached under an option that shows no slots', () => {
		const deferred = {
			...complete(),
			thumbnailMode: '3',
			thumbnails: [{ hash: 'b'.repeat(64), kind: 'image' as const, byte_len: 10 }]
		};
		expect(refusalsIn(refusalsOf(deferred, VOCABULARY), 'files')).toHaveLength(1);
	});
});

describe('the additional-licence pre-fill', () => {
	it('is ninety percent of the price, rounded down to the cent', () => {
		expect(suggestedAdditionalLicence('4.50', 90)).toBe('4.05');
		expect(suggestedAdditionalLicence('0.99', 90)).toBe('0.89');
	});

	it('says nothing where no price has been typed', () => {
		expect(suggestedAdditionalLicence('', 90)).toBe('');
		expect(minorUnitsOf('4.501')).toBeNull();
	});
});

describe('the per-marketplace divergence', () => {
	it('follows the canonical value until the marketplace value is edited', () => {
		const draft = { ...complete(), inventories: ['Tpt' as const, 'TesGb' as const] };
		expect(valueFor(draft, 'TesGb', 'name')).toBe(draft.name);
		expect(diverges(draft, 'TesGb', 'name')).toBe(false);
		expect(divergentOn(draft, 'name')).toEqual([]);
	});

	it('offers Update all only once a marketplace value differs', () => {
		const draft = withOverride(
			{ ...complete(), inventories: ['Tpt', 'TesGb'] },
			'TesGb',
			'name',
			'Fractions — UK edition'
		);
		expect(diverges(draft, 'TesGb', 'name')).toBe(true);
		expect(divergentOn(draft, 'name')).toEqual(['TesGb']);
		expect(valueFor(draft, 'Tpt', 'name')).toBe(complete().name);
	});

	it('Reset drops the override rather than emptying the field', () => {
		let draft = withOverride({ ...complete(), inventories: ['TesGb'] }, 'TesGb', 'name', 'Other');
		draft = withOverride(draft, 'TesGb', 'name', null);
		expect(diverges(draft, 'TesGb', 'name')).toBe(false);
		expect(valueFor(draft, 'TesGb', 'name')).toBe(complete().name);
	});

	it('Update all promotes one value and clears every override of that field', () => {
		let draft: TptDraft = { ...complete(), inventories: ['Tpt', 'TesGb'] };
		draft = withOverride(draft, 'TesGb', 'name', 'Fractions — UK edition');
		draft = applyToAll(draft, 'name', 'Fractions — UK edition');
		expect(draft.name).toBe('Fractions — UK edition');
		expect(divergentOn(draft, 'name')).toEqual([]);
		expect(valueFor(draft, 'Tpt', 'name')).toBe('Fractions — UK edition');
	});
});

describe('the request bodies', () => {
	it('sends every group to the check endpoint, ids and not menu positions', () => {
		const draft = { ...complete(), answerKey: '4', teachingDuration: '1', pagesOrSlides: '12' };
		const input = draftInputOf(draft);
		expect(input.answer_key_id).toBe(4);
		expect(labelOf(VOCABULARY.answer_keys, '4')).toBe('Included with Rubric');
		expect(input.teaching_duration_id).toBe(1);
		expect(input.pages_or_slides).toBe(12);
		expect(input.copyright_declaration_id).toBe(1);
		expect(input.grades).toEqual(['3rd-grade']);
	});

	it('sends no tax code on a free listing, because the control is not shown', () => {
		expect(draftInputOf(complete()).tax_code_id).toBeNull();
		expect(draftInputOf(complete()).price_minor_units).toBeNull();
	});

	it('carries the grades onto the create as verbatim TPT paths', () => {
		const body = createBodyOf(complete());
		expect(body?.grades).toEqual([
			{ inventory: 'Tpt', kind: 'phase', segments: ['3rd-grade'], native_id: '3rd-grade' }
		]);
		expect(body?.price).toBe('Free');
	});

	it('sends every remaining group in the sidecar block, and no field twice', () => {
		const draft = {
			...complete(),
			tags: ['centers'],
			formats: ['easel'],
			customCategories: ['Autumn unit'],
			answerKey: '4',
			teachingDuration: '1',
			pagesOrSlides: '12'
		};
		const base = tptBaseOf(draft);
		expect(base.subject_areas).toEqual(['math']);
		expect(base.tags).toEqual(['centers']);
		expect(base.formats).toEqual(['easel']);
		expect(base.custom_categories).toEqual(['Autumn unit']);
		expect(base.answer_key_id).toBe(4);
		expect(base.copyright_declaration_id).toBe(1);
		expect(base.status_user).toBe(0);
		const body = createBodyOf(draft);
		expect(body?.tpt_base).toEqual(base);
		// The title, description, price, payload and grades travel on the body
		// itself, so the block carries no second copy of any of them.
		expect(Object.keys(base)).not.toContain('name');
		expect(Object.keys(base)).not.toContain('grades');
	});

	it('carries the localisation flag in the sidecar block and nowhere else', () => {
		const body = createBodyOf({ ...complete(), appropriateForCountry: true });
		expect(body?.tpt_base?.appropriate_for_country).toBe(true);
		// Serialised with the block removed rather than checked key by key: a
		// named-key assertion would pass an implementation that promoted the
		// flag onto some other part of the body, and promoting it anywhere on
		// the body changes a wire that migration 0040's sidecar already holds.
		expect(JSON.stringify({ ...body, tpt_base: null })).not.toContain(
			'appropriate_for_country'
		);
	});

	it('sends the flag unticked on a blank draft, which is what TPT’s own checkbox posts', () => {
		expect(emptyTptDraft().appropriateForCountry).toBe(false);
		expect(tptBaseOf(complete()).appropriate_for_country).toBe(false);
	});

	it('sends no tax code or licence price on a free listing', () => {
		const base = tptBaseOf(complete());
		expect(base.tax_code_id).toBeNull();
		expect(base.additional_licence_minor_units).toBeNull();
		expect(base.bundle_discount_minor_units).toBeNull();
	});

	it('sends nothing where the price or the payload is missing', () => {
		expect(createBodyOf({ ...complete(), payload: [] })).toBeNull();
		expect(createBodyOf({ ...complete(), free: false, price: '' })).toBeNull();
	});
});

describe('the marketplace tab contract', () => {
	it('renders one field row per overridable field, decided by the listing', () => {
		const projection = projectionOf({ ...complete(), inventories: ['TesGb'] }, 'TesGb');
		expect(projection.inventory).toBe('TesGb');
		expect(
			projection.rows.map((row) => [row.key, row.kind, row.decided_by?.by])
		).toEqual([
			['name', 'field', 'listing'],
			['description', 'field', 'listing'],
			['price', 'field', 'listing']
		]);
		expect(projection.rows[0].values).toEqual([complete().name]);
	});

	it('marks a row the seller took away from the canonical value', () => {
		const draft = withOverride(
			{ ...complete(), inventories: ['TesGb'] },
			'TesGb',
			'name',
			'Fractions — UK edition'
		);
		const row = projectionOf(draft, 'TesGb').rows[0];
		expect(row.decided_by?.by).toBe('listing_override');
		expect(row.values).toEqual(['Fractions — UK edition']);
	});

	/** The row type is the shape the planned per-organisation override slice
	 *  fills, so a server-supplied axis row renders in the same tab without
	 *  this file changing. */
	it('carries a shape a server-side axis row can be fed into unchanged', () => {
		const fromServer: MarketplaceProjection = {
			inventory: 'TesGb',
			rows: [
				{
					key: 'subject',
					kind: 'axis',
					label: 'Subject',
					values: ['mathematics', 'numeracy'],
					decided_by: { by: 'override', decided_at: '2026-09-03T00:00:00Z' },
					loss: null
				}
			]
		};
		expect(fromServer.rows[0].decided_by).toEqual({
			by: 'override',
			decided_at: '2026-09-03T00:00:00Z'
		});
		expect(fromServer.rows[0].values).toHaveLength(2);
	});
});

/** One marketplace's served vocabulary, trimmed to the axes these tests read.
 *
 *  The bindings are Tes's own, from `crates/tam-domain/src/registry/tes.rs:205-234`:
 *  subject and phase are many-valued and delegable by opt-in, and licence is
 *  one-valued and never delegable, which is the legal-content refusal declared
 *  as data. The subject cap is the one invented value here and it is a fixture
 *  rather than a claim: no Tes cap has been measured, and the registry says so
 *  by leaving it absent. It is set to exercise the rule that a set over a cap
 *  is disclosed and never narrowed. */
function vocabularyWithSubjectCap(cap: number | undefined): VocabularyView {
	return {
		inventory: 'TesGb',
		marketplace: 'Tes',
		canonical: [],
		natives: [],
		axes: [
			{
				axis: 'subject',
				native: 'categories',
				cardinality: 'many',
				cap,
				delegation: { kind: 'by_opt_in' },
				required: false
			},
			{
				axis: 'phase',
				native: 'ageRange',
				cardinality: 'many',
				delegation: { kind: 'by_opt_in' },
				required: false
			},
			{
				axis: 'licence',
				native: 'licence',
				cardinality: 'one',
				delegation: { kind: 'never', reason: 'legal_content' },
				required: true
			}
		],
		absent_axes: [],
		authoring: {
			payload_files: 'every_payload_file',
			body_wire: 'carries_declared_format',
			body_formats: ['Html']
		}
	};
}

function axisRow(projection: MarketplaceProjection, axis: string) {
	return projection.rows.find((row) => row.kind === 'axis' && row.key === axis);
}

describe('the axis rows on a marketplace tab', () => {
	it('discloses a cap the listing exceeds and narrows nothing to fit it', () => {
		const draft = {
			...complete(),
			subjectAreas: ['math', 'science', 'social-studies'],
			inventories: ['TesGb' as const]
		};
		const row = axisRow(projectionOf(draft, 'TesGb', vocabularyWithSubjectCap(2)), 'subject');
		expect(row?.loss).toContain('takes 2');
		expect(row?.loss).toContain('chosen 3');
		// The whole point: the seller's three survive on the row and the row
		// carries no two-element value that a later reader could mistake for
		// what they chose. A truncating implementation passes every other
		// assertion here and fails this one.
		expect(row?.axis?.stated).toEqual(['math', 'science', 'social-studies']);
		expect(row?.values).toEqual([]);
	});

	it('discloses nothing where no cap is measured, because absent is not unlimited', () => {
		const draft = {
			...complete(),
			subjectAreas: ['math', 'science', 'social-studies'],
			inventories: ['TesGb' as const]
		};
		const row = axisRow(projectionOf(draft, 'TesGb', vocabularyWithSubjectCap(undefined)), 'subject');
		expect(row?.loss).toBeNull();
		expect(row?.axis?.cap).toBeNull();
	});

	it('reads a one-valued axis as a cap of one rather than as a separate case', () => {
		const draft = { ...complete(), inventories: ['TesGb' as const] };
		expect(axisRow(projectionOf(draft, 'TesGb', vocabularyWithSubjectCap(3)), 'licence')?.axis?.cap)
			.toBe(1);
	});

	it('refuses the licence axis an override and any computed answer', () => {
		const draft = { ...complete(), inventories: ['TesGb' as const] };
		const projection = projectionOf(draft, 'TesGb', vocabularyWithSubjectCap(3));
		const licence = axisRow(projection, 'licence');
		expect(licence?.axis?.delegable).toBe(false);
		expect(licence?.axis?.mode).toBe('seller_decides');
		expect(licence?.values).toEqual([]);
		// Not vacuous: a delegable axis on the same tab reads the other way, so
		// an implementation that marked every axis undelegable fails here.
		expect(axisRow(projection, 'subject')?.axis?.delegable).toBe(true);
		expect(axisRow(projection, 'subject')?.axis?.mode).toBe('best_fit');
		// Overrides are keyed by field and the licence axis is not one of them,
		// which is what stops the tab offering a control for it.
		expect(OVERRIDABLE.map((entry) => entry.field)).not.toContain('licence');
	});

	it('decides no axis in the browser, whatever the mode says', () => {
		const draft = { ...complete(), inventories: ['TesGb' as const] };
		const rows = projectionOf(draft, 'TesGb', vocabularyWithSubjectCap(3)).rows.filter(
			(row) => row.kind === 'axis'
		);
		expect(rows.length).toBeGreaterThan(0);
		for (const row of rows) {
			expect(row.values).toEqual([]);
			expect(row.decided_by).toBeNull();
		}
	});

	it('renders no axis row at all until the marketplace vocabulary has arrived', () => {
		const draft = { ...complete(), inventories: ['TesGb' as const] };
		expect(projectionOf(draft, 'TesGb', null).rows.every((row) => row.kind === 'field')).toBe(true);
	});
});
