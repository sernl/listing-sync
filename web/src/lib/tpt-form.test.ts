import { describe, expect, it } from 'vitest';
import { loadCoreForTest } from '$lib/core/testing';
import {
	applyToAll,
	atCap,
	capOf,
	counterOf,
	createBodyOf,
	createdToast,
	diverges,
	divergentOn,
	draftInputOf,
	emptySlots,
	emptyTptDraft,
	gradeColumns,
	labelOf,
	licenceIntentOf,
	minorUnitsOf,
	needsFileBeforeMarketplace,
	OVERRIDABLE,
	projectionOf,
	standardsLoss,
	refusalsOf,
	shouldLandOnCreated,
	slotsSettling,
	thumbnailHandles,
	thumbnailRefusal,
	unlicensed,
	standardsHelp,
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

	it('lets a finished resource name no marketplace at all', () => {
		const kept = { ...complete(), inventories: [] };
		expect(refusalsOf(kept, VOCABULARY)).toEqual([]);
		expect(submittable(refusalsOf(kept, VOCABULARY))).toBe(true);
	});

	it('refuses a marketplace chosen before the file it would carry', () => {
		const early = { ...complete(), payload: [] };
		expect(refusalsIn(refusalsOf(early, VOCABULARY), 'product_status')[0].message).toBe(
			'Add your file before sending this to a marketplace.'
		);
	});
});

describe('the warning that a marketplace needs a file first', () => {
	// The condition the dialog opens on, held here rather than in the markup:
	// a rule reachable only by rendering the page is a rule nothing cheap can
	// check, which is what `sweep-triage.md` asks of a substitution like this.

	it('opens once a marketplace is chosen and no file is uploaded', () => {
		expect(needsFileBeforeMarketplace({ ...complete(), payload: [] })).toBe(true);
	});

	it('stays shut for a draft kept here, which is the case it must not block', () => {
		expect(needsFileBeforeMarketplace({ ...complete(), payload: [], inventories: [] })).toBe(
			false
		);
	});

	it('stays shut once the file is uploaded', () => {
		expect(needsFileBeforeMarketplace(complete())).toBe(false);
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

	it('states the flag on save whichever way the box is ticked', () => {
		// The wire's absent case means a product nobody asked, which is what a
		// row predating the column is. A seller looking at the control has been
		// asked, so both answers are stated and neither arrives as absence.
		expect(emptyTptDraft().appropriateForCountry).toBe(false);
		for (const ticked of [true, false]) {
			const draft = { ...complete(), appropriateForCountry: ticked };
			expect(tptBaseOf(draft).appropriate_for_country).toBe(ticked);
			expect(draftInputOf(draft).appropriate_for_country).toBe(ticked);
		}
	});

	it('sends no tax code or licence price on a free listing', () => {
		const base = tptBaseOf(complete());
		expect(base.tax_code_id).toBeNull();
		expect(base.additional_licence_minor_units).toBeNull();
		expect(base.bundle_discount_minor_units).toBeNull();
	});

	it('sends nothing where the price is one this client will not send', () => {
		// The payload is no longer part of this: a resource kept here carries no
		// file and is composed all the same (D32). Only an unsendable price
		// stops a body being built.
		expect(createBodyOf({ ...complete(), free: false, price: '' })).toBeNull();
		expect(createBodyOf({ ...complete(), payload: [] })).not.toBeNull();
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

describe('the standards a marketplace will not carry', () => {
	function pick(code: string, nodeId?: number, guid = `guid-${code}`) {
		return {
			framework: 3054,
			code,
			statement: `${code} statement`,
			source_guid: guid,
			tpt_node_id: nodeId
		};
	}

	it('says nothing where every pick can be posted', () => {
		expect(standardsLoss([pick('A.1', 11), pick('A.2', 12)])).toBeNull();
	});

	it('says nothing at all when nothing was picked', () => {
		expect(standardsLoss([])).toBeNull();
	});

	/// The case that is true of every standard today, because the node-id table
	/// is committed empty: a seller sees which of their picks stay behind.
	it('names each pick that stays in the catalogue, and how many of how many', () => {
		const loss = standardsLoss([pick('A.1', 11), pick('A.2'), pick('A.3')]);
		expect(loss).toContain('2 of 3');
		expect(loss).toContain('A.2');
		expect(loss).toContain('A.3');
		expect(loss).not.toContain('A.1');
	});

	it('reads singly for one pick and plurally for several', () => {
		expect(standardsLoss([pick('A.2')])).toContain('It stays');
		expect(standardsLoss([pick('A.2'), pick('A.3')])).toContain('They stay');
	});

	it('puts the loss on a standards row of the TPT tab', () => {
		const draft = {
			...complete(),
			standards: [pick('A.1', 11), pick('A.2')]
		};
		const row = projectionOf(draft, 'Tpt').rows.find((one) => one.key === 'standards');
		expect(row).toBeDefined();
		expect(row?.label).toBe('Standards');
		expect(row?.values).toEqual(['A.1', 'A.2']);
		expect(row?.loss).toContain('1 of 2');
	});

	/// No Tes or Etsy field takes a standard, so a standards row on their tab
	/// would disclose a loss that is not one.
	it('renders no standards row on a marketplace that carries none', () => {
		const draft = {
			...complete(),
			inventories: ['TesGb' as const],
			standards: [pick('A.1'), pick('A.2')]
		};
		expect(
			projectionOf(draft, 'TesGb').rows.find((one) => one.key === 'standards')
		).toBeUndefined();
	});

	it('renders no standards row where the seller picked none', () => {
		expect(
			projectionOf(complete(), 'Tpt').rows.find((one) => one.key === 'standards')
		).toBeUndefined();
	});

	/// The reason a pick is identified by the mirror's guid rather than by its
	/// code: 814 TEKS codes name more than one addressable node, so two picks
	/// can share a code and mean different standards. The loss must count both.
	it('counts two standards that share a code as two', () => {
		const loss = standardsLoss([
			pick('1.1.A', undefined, 'guid-maths'),
			pick('1.1.A', undefined, 'guid-science')
		]);
		expect(loss).toContain('2 of 2');
		expect(loss).toContain('They stay');
	});
});

describe('the standards heading', () => {
	it('names the count the server served', () => {
		expect(standardsHelp(4)).toBe('Optional. Four frameworks, each searched on its own.');
		expect(standardsHelp(2)).toBe('Optional. Two frameworks, each searched on its own.');
	});

	it('says nothing at all when none is offered', () => {
		// The sentence used to read "Four frameworks" over a panel saying none
		// was offered at all, whenever the vocabulary served an empty list. It
		// says nothing now rather than a shorter version of what the picker
		// below already says at more length.
		expect(standardsHelp(0)).toBeUndefined();
		expect(standardsHelp(-1)).toBeUndefined();
	});

	it('does not say "each" of one', () => {
		expect(standardsHelp(1)).toBe('Optional. One framework.');
	});

	it('falls back to the digit above the words it spells', () => {
		expect(standardsHelp(12)).toBe('Optional. 12 frameworks, each searched on its own.');
	});
});

/** A marketplace that gates a licence, as `GET /v1/vocabulary/{inventory}`
 *  serves one. Transcribed from `crates/tam-domain/src/registry/tes.rs`, where
 *  `licence` is the one field declared required anywhere in the registry. */
function tesGating(): VocabularyView {
	return {
		inventory: 'TesGb',
		marketplace: 'Tes',
		canonical: [],
		natives: [
			{
				name: 'licence',
				direction: 'both',
				required: true,
				vocabulary: 'closed',
				values: [
					{ id: 'CC-BY', label: 'Creative Commons Attribution' },
					{ id: 'TES-PAID', label: 'Tes paid licence' }
				],
				delegation: { kind: 'never', reason: 'legal_content' }
			}
		],
		axes: [],
		absent_axes: [],
		authoring: {
			payload_files: 'every_payload_file',
			body_wire: 'carries_declared_format',
			body_formats: ['Html'],
			licence: { native: 'licence', free: ['CC-BY'], paid: ['TES-PAID'] }
		}
	} as VocabularyView;
}

const GATING = new Map([['TesGb' as const, tesGating()]]);

describe('the licence a marketplace gates', () => {
	// The defect this pins: `createBodyOf` sent `elections: []` and no `rights`,
	// so `required_fields_answered` on the server refused every Tes create and
	// no Tes listing could be made from this form at all.

	it('carries the seller’s licence as a rights grant and an election', () => {
		const listing = {
			...complete(),
			inventories: ['TesGb' as const],
			free: true,
			licence: 'CC-BY'
		};
		const body = createBodyOf(listing, GATING);
		expect(body?.rights).toEqual({
			inventory: 'TesGb',
			segments: ['CC-BY'],
			native_id: 'CC-BY'
		});
		expect(body?.elections).toEqual([
			{
				inventory: 'TesGb',
				axis: 'licence',
				trigger: 'supply',
				trigger_key: 'free',
				answers: [{ segments: ['CC-BY'], native_id: 'CC-BY' }]
			}
		]);
	});

	it('sends the paid branch’s key when the listing carries a price', () => {
		const paid = {
			...complete(),
			inventories: ['TesGb' as const],
			free: false,
			price: '4.50',
			taxCode: '1',
			licence: 'TES-PAID'
		};
		expect(licenceIntentOf(paid).branch).toBe('paid');
		expect(createBodyOf(paid, GATING)?.elections?.[0].trigger_key).toBe('paid');
	});

	it('refuses a gating marketplace with no licence chosen, naming it', () => {
		const bare = { ...complete(), inventories: ['TesGb' as const], free: true, licence: null };
		expect(unlicensed(bare, GATING)).toEqual(['TesGb']);
		const said = refusalsIn(refusalsOf(bare, VOCABULARY, GATING), 'product_status');
		expect(said).toHaveLength(1);
		expect(said[0].message).toContain('Choose a licence');
	});

	it('asks for no licence where no chosen marketplace gates one', () => {
		const tpt = { ...complete(), inventories: ['Tpt' as const], free: true, licence: null };
		expect(unlicensed(tpt, GATING)).toEqual([]);
		expect(refusalsOf(tpt, VOCABULARY, GATING)).toEqual([]);
	});

	it('composes neither shape for a listing that names no marketplace', () => {
		const kept = { ...complete(), inventories: [], free: true, licence: null };
		const body = createBodyOf(kept, GATING);
		expect(body?.rights).toBeNull();
		expect(body?.elections).toEqual([]);
	});
});

describe('the four thumbnail slots', () => {
	// They collected nothing until now, and the page said so. What changed is
	// the reading of `POST /{version}/uploads`: it takes one file and answers
	// with that file's own handle, so a slot is named by which request the
	// client made rather than by anything the wire carries.

	function handle(mark: string) {
		return { hash: mark.repeat(64), kind: 'image' as const, byte_len: 2048 };
	}

	it('starts with four empty slots', () => {
		expect(emptySlots()).toHaveLength(4);
		expect(thumbnailHandles(emptySlots())).toEqual([]);
	});

	it('carries only the slots the upload answered for, in slot order', () => {
		const slots = emptySlots();
		slots[0] = { local: 'blob:a', handle: handle('a'), sending: false, refusal: null };
		// Chosen and drawn, but still in flight: the picture is on the page and
		// the handle is not, and only the handle may reach the wire.
		slots[1] = { local: 'blob:b', handle: null, sending: true, refusal: null };
		slots[2] = { local: 'blob:c', handle: handle('c'), sending: false, refusal: null };
		expect(thumbnailHandles(slots).map((file) => file.hash[0])).toEqual(['a', 'c']);
	});

	it('holds the create back while any slot is still being sent', () => {
		const slots = emptySlots();
		expect(slotsSettling(slots)).toBe(false);
		slots[2] = { local: 'blob:c', handle: null, sending: true, refusal: null };
		expect(slotsSettling(slots)).toBe(true);
	});

	it('sends the handles it holds in the sidecar block', () => {
		const listing = { ...complete(), thumbnailMode: '2', thumbnails: [handle('a'), handle('c')] };
		expect(tptBaseOf(listing).thumbnail_hashes).toEqual([
			'a'.repeat(64),
			'c'.repeat(64)
		]);
	});
});

describe('what the seller is told once the draft exists', () => {
	// Found live: an empty mapping list fell through to the plural arm, so a
	// resource kept here was announced as "Draft created on 0 marketplaces.
	// Publish when you are ready." — a wrong count, and an instruction to
	// publish something deliberately going nowhere.

	it('says the draft is kept here when no marketplace was chosen', () => {
		expect(createdToast(0)).toBe('Draft saved here. Choose marketplaces when you are ready.');
		expect(createdToast(0)).not.toContain('0 marketplaces');
		expect(createdToast(0)).not.toContain('Publish');
	});

	it('counts one marketplace as a word and several as a figure', () => {
		expect(createdToast(1)).toContain('one marketplace');
		expect(createdToast(3)).toContain('3 marketplaces');
	});
});

describe('a resource kept here with no file', () => {
	// D32 on the client: the body is composed, because the server now accepts
	// it. Before this the form declined to build one at all, so the founder's
	// case could not have been sent even once the server allowed it.

	it('composes a create body with no file and no marketplace', () => {
		const kept = { ...complete(), payload: [], inventories: [], free: true };
		const body = createBodyOf(kept);
		expect(body).not.toBeNull();
		expect(body?.payload).toEqual([]);
		expect(body?.inventories).toEqual([]);
	});

	it('is submittable, so the seller can actually keep it', () => {
		const kept = { ...complete(), payload: [], inventories: [], free: true };
		expect(refusalsOf(kept, VOCABULARY)).toEqual([]);
	});

	it('still refuses to compose a price this client will not send', () => {
		const unsendable = { ...complete(), payload: [], inventories: [], free: false, price: 'free' };
		expect(createBodyOf(unsendable)).toBeNull();
	});
});

describe('where the seller lands after a create', () => {
	// A create takes seconds against a real server. `goto` after the response
	// yanked a seller who had already moved on back onto a detail page they had
	// not asked for, six times in six.

	it('lands on the new resource when the seller is still on the form', () => {
		expect(shouldLandOnCreated('/inventory/new', '/inventory/new')).toBe(true);
	});

	it('leaves the seller where they went when they navigated away mid-create', () => {
		expect(shouldLandOnCreated('/inventory/new', '/analytics')).toBe(false);
		expect(shouldLandOnCreated('/inventory/new', '/inventory')).toBe(false);
	});
});

describe('what a slot will not take as a thumbnail', () => {
	const cap = 4 * 1024 * 1024;

	it('accepts a picture inside the limit', () => {
		expect(thumbnailRefusal('image/png', 2048, cap)).toBeNull();
		expect(thumbnailRefusal('image/jpeg', cap, cap)).toBeNull();
	});

	it('refuses anything that is not a picture, however it arrived', () => {
		// `accept="image/*"` is a picker hint and does not survive a drop.
		expect(thumbnailRefusal('application/pdf', 2048, cap)).toMatch(/have to be pictures/);
		expect(thumbnailRefusal('', 2048, cap)).toMatch(/have to be pictures/);
	});

	it('refuses a picture over the limit and states both figures', () => {
		const said = thumbnailRefusal('image/png', 12 * 1024 * 1024, cap);
		expect(said).toContain('12 MB');
		expect(said).toContain('4 MB');
	});
});
