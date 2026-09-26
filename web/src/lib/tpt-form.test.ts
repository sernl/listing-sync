import { describe, expect, it } from 'vitest';
import { loadCoreForTest } from '$lib/core/testing';
import {
	advisoriesOf,
	applyToAll,
	atCap,
	capOf,
	counterOf,
	createBodyOf,
	createdToast,
	diverges,
	divergentOn,
	draftInputOf,
	draftOf,
	emptySlots,
	emptyTptDraft,
	gradeColumns,
	labelOf,
	licenceIntentOf,
	minorUnitsOf,
	needsFileBeforeMarketplace,
	patchBodyOf,
	withoutPayload,
	slotsFrom,
	OVERRIDABLE,
	projectionOf,
	standardsLoss,
	refusalsOf,
	shouldLandOnCreated,
	slotsSettling,
	thumbnailHashes,
	thumbnailRefusal,
	unlicensed,
	STANDARDS_HELP,
	marketplacesReached,
	refusalsIn,
	searchFacets,
	submittable,
	suggestedAdditionalLicence,
	togglePick,
	tptBaseOf,
	valueFor,
	withOverride,
	withMarketplaces,
	inventoriesOf,
	tilesOf,
	gradeLabel,
	gradeBandLabel,
	markUp,
	payloadOf,
	type StoredFile,
	type MarketplaceProjection,
	type TptDraft
} from './tpt-form';
import type {
	CreateProductBody,
	FacetView,
	FormVocabularyView,
	ProductView,
	TptBaseView,
	VocabularyView
} from '$lib/api';

// The refusals below are the compiled core's, not this file's, so the module
// has to be in memory before any of them is asked for.
loadCoreForTest();

/** The vocabulary as `GET /v1/authoring/vocabulary` serves it, trimmed to the
 *  fields these tests read. Transcribed from `crates/tam-api/src/product/mod.rs`
 *  and the committed capture rather than invented; the caps are the numbers
 *  `TptForm` reads, including the absent subject-area one. */
function facet(
	slug: string,
	label: string,
	writable = true,
	british: string | null = null
): FacetView {
	return { slug, label, seller_writable: writable, british_label: british };
}

const VOCABULARY: FormVocabularyView = {
	grades: [
		facet('preschool', 'Preschool'),
		facet('kindergarten', 'Kindergarten', true, 'Year 1'),
		facet('1st-grade', '1st Grade', true, 'Year 2'),
		facet('2nd-grade', '2nd Grade', true, 'Year 3'),
		facet('3rd-grade', '3rd Grade', true, 'Year 4'),
		facet('4th-grade', '4th Grade', true, 'Year 5'),
		facet('5th-grade', '5th Grade', true, 'Year 6'),
		facet('6th-grade', '6th Grade', true, 'Year 7'),
		facet('7th-grade', '7th Grade', true, 'Year 8'),
		facet('8th-grade', '8th Grade', true, 'Year 9'),
		facet('9th-grade', '9th Grade', true, 'Year 10'),
		facet('10th-grade', '10th Grade', true, 'Year 11'),
		facet('11th-grade', '11th Grade', true, 'Year 12'),
		facet('12th-grade', '12th Grade', true, 'Year 13'),
		facet('higher-education', 'Higher Education'),
		facet('adult-education', 'Adult Education'),
		facet('not-grade-specific', 'Not Grade Specific'),
		facet('elementary', 'Elementary', false, 'Primary School'),
		facet('middle-school', 'Middle School', false, 'Secondary School'),
		facet('high-school', 'High School', false, 'College')
	],
	grade_columns: [5, 5, 4, 3],
	grade_bands: [
		{ american: 'Elementary', british: 'Primary School' },
		{ american: 'Middle School', british: 'Secondary School' },
		{ american: 'High School', british: 'College' },
		{ american: 'Other', british: 'Other' }
	],
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
		{ id: '1', label: 'Make listing active on my selected marketplaces' }
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
	// Through `withMarketplaces`, because that is the only way the form writes
	// the two marketplace fields: a fixture that set `inventories` alone
	// would be a draft the page cannot produce.
	return withMarketplaces(
		{
			...emptyTptDraft(),
			name: 'Fractions on a number line',
			payload: [{ hash: 'a'.repeat(64), kind: 'pdf', byte_len: 1024 }],
			free: true,
			grades: ['3rd-grade'],
			subjectAreas: ['math'],
			tags: ['centers'],
			copyright: '1'
		},
		['Tpt']
	);
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
		// The attestation is asked in the TPT-only panel now, so the refusal
		// about it has to land there rather than on a Copyright heading that is
		// no longer on the page.
		expect(refusalsIn(refusals, 'tpt_options')).toHaveLength(1);
		expect(refusalsIn(refusals, 'tpt_options')[0].message).toBe(
			'Choose one of the two copyright statements.'
		);
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
		expect(refusal.message).toBe('Choose up to 4 under Grade Level; you have 5.');
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
		// The tax code is asked in the TPT-only panel, so its refusal points
		// there rather than into the middle of Price.
		expect(refusalsIn(refusalsOf(paid, VOCABULARY), 'tpt_options').map((r) => r.control)).toEqual(
			['Tax Code']
		);
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
			thumbnails: ['b'.repeat(64)]
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
		expect(refusalsIn(refusalsOf(early, VOCABULARY), 'marketplaces')[0].message).toBe(
			'Add your file before you choose a marketplace.'
		);
	});
});

describe('a vocabulary id the core’s own range does not hold', () => {
	// The wave-5 render fixture offered tax codes with invented ids, 81111
	// and 81112, and choosing one threw `invalid value: integer 81111,
	// expected u8` out of the core inside the page's own `$derived`, so the
	// whole resource page drew its error boundary. The id is now one refusal
	// naming the one control, and everything else the page derives from the
	// core still stands.
	const invented = { ...complete(), free: false, price: '4.50', taxCode: '81111' };

	it('travels to the core as the number the form holds', () => {
		expect(draftInputOf(invented).tax_code_id).toBe(81111);
	});

	it('is refused by the control that holds it rather than thrown', () => {
		const refusals = refusalsOf(invented, VOCABULARY);
		expect(submittable(refusals)).toBe(false);
		expect(refusalsIn(refusals, 'tpt_options').map((r) => r.control)).toEqual(['Tax Code']);
		expect(refusalsIn(refusals, 'tpt_options')[0].message).toBe('Choose a tax code from the list.');
	});

	it('leaves the rest of the view model standing', () => {
		expect(advisoriesOf(invented, VOCABULARY)).toEqual([]);
		expect(projectionOf(invented, 'Tpt').rows.length).toBeGreaterThan(0);
		expect(refusalsIn(refusalsOf(invented, VOCABULARY), 'categories')).toEqual([]);
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
		const draft = { ...complete(), inventories: ['Tpt' as const, 'Tes' as const] };
		expect(valueFor(draft, 'Tes', 'name')).toBe(draft.name);
		expect(diverges(draft, 'Tes', 'name')).toBe(false);
		expect(divergentOn(draft, 'name')).toEqual([]);
	});

	it('offers Update all only once a marketplace value differs', () => {
		const draft = withOverride(
			{ ...complete(), inventories: ['Tpt', 'Tes'] },
			'Tes',
			'name',
			'Fractions — UK edition'
		);
		expect(diverges(draft, 'Tes', 'name')).toBe(true);
		expect(divergentOn(draft, 'name')).toEqual(['Tes']);
		expect(valueFor(draft, 'Tpt', 'name')).toBe(complete().name);
	});

	it('Reset drops the override rather than emptying the field', () => {
		let draft = withOverride({ ...complete(), inventories: ['Tes'] }, 'Tes', 'name', 'Other');
		draft = withOverride(draft, 'Tes', 'name', null);
		expect(diverges(draft, 'Tes', 'name')).toBe(false);
		expect(valueFor(draft, 'Tes', 'name')).toBe(complete().name);
	});

	it('Update all promotes one value and clears every override of that field', () => {
		let draft: TptDraft = { ...complete(), inventories: ['Tpt', 'Tes'] };
		draft = withOverride(draft, 'Tes', 'name', 'Fractions — UK edition');
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
		const projection = projectionOf({ ...complete(), inventories: ['Tes'] }, 'Tes');
		expect(projection.inventory).toBe('Tes');
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
			{ ...complete(), inventories: ['Tes'] },
			'Tes',
			'name',
			'Fractions — UK edition'
		);
		const row = projectionOf(draft, 'Tes').rows[0];
		expect(row.decided_by?.by).toBe('listing_override');
		expect(row.values).toEqual(['Fractions — UK edition']);
	});

	/** The row type is the shape the planned per-organisation override slice
	 *  fills, so a server-supplied axis row renders in the same tab without
	 *  this file changing. */
	it('carries a shape a server-side axis row can be fed into unchanged', () => {
		const fromServer: MarketplaceProjection = {
			inventory: 'Tes',
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
		inventory: 'Tes',
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
			inventories: ['Tes' as const]
		};
		const row = axisRow(projectionOf(draft, 'Tes', vocabularyWithSubjectCap(2)), 'subject');
		expect(row?.loss).toContain('takes 2');
		expect(row?.loss).toContain('chose 3');
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
			inventories: ['Tes' as const]
		};
		const row = axisRow(projectionOf(draft, 'Tes', vocabularyWithSubjectCap(undefined)), 'subject');
		expect(row?.loss).toBeNull();
		expect(row?.axis?.cap).toBeNull();
	});

	it('reads a one-valued axis as a cap of one rather than as a separate case', () => {
		const draft = { ...complete(), inventories: ['Tes' as const] };
		expect(axisRow(projectionOf(draft, 'Tes', vocabularyWithSubjectCap(3)), 'licence')?.axis?.cap)
			.toBe(1);
	});

	it('refuses the licence axis an override and any computed answer', () => {
		const draft = { ...complete(), inventories: ['Tes' as const] };
		const projection = projectionOf(draft, 'Tes', vocabularyWithSubjectCap(3));
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
		const draft = { ...complete(), inventories: ['Tes' as const] };
		const rows = projectionOf(draft, 'Tes', vocabularyWithSubjectCap(3)).rows.filter(
			(row) => row.kind === 'axis'
		);
		expect(rows.length).toBeGreaterThan(0);
		for (const row of rows) {
			expect(row.values).toEqual([]);
			expect(row.decided_by).toBeNull();
		}
	});

	it('renders no axis row at all until the marketplace vocabulary has arrived', () => {
		const draft = { ...complete(), inventories: ['Tes' as const] };
		expect(projectionOf(draft, 'Tes', null).rows.every((row) => row.kind === 'field')).toBe(true);
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
			inventories: ['Tes' as const],
			standards: [pick('A.1'), pick('A.2')]
		};
		expect(
			projectionOf(draft, 'Tes').rows.find((one) => one.key === 'standards')
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
	// The count is gone: the founder's rule is one short sentence that says
	// what to do, and how many frameworks there are is something the tab bar
	// below already shows.
	it('tells the teacher what to do rather than counting frameworks', () => {
		expect(STANDARDS_HELP).toBe('Optional. Search a framework by code or words.');
	});
});

/** A marketplace that gates a licence, as `GET /v1/vocabulary/{inventory}`
 *  serves one. Transcribed from `crates/tam-domain/src/registry/tes.rs`, where
 *  `licence` is the one field declared required anywhere in the registry. */
function tesGating(): VocabularyView {
	return {
		inventory: 'Tes',
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

const GATING = new Map([['Tes' as const, tesGating()]]);

describe('the licence a marketplace gates', () => {
	// The defect this pins: `createBodyOf` sent `elections: []` and no `rights`,
	// so `required_fields_answered` on the server refused every Tes create and
	// no Tes listing could be made from this form at all.

	it('carries the seller’s licence as a rights grant and an election', () => {
		const listing = {
			...complete(),
			inventories: ['Tes' as const],
			free: true,
			licence: 'CC-BY'
		};
		const body = createBodyOf(listing, GATING);
		expect(body?.rights).toEqual({
			inventory: 'Tes',
			segments: ['CC-BY'],
			native_id: 'CC-BY'
		});
		expect(body?.elections).toEqual([
			{
				inventory: 'Tes',
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
			inventories: ['Tes' as const],
			free: false,
			price: '4.50',
			taxCode: '1',
			licence: 'TES-PAID'
		};
		expect(licenceIntentOf(paid).branch).toBe('paid');
		expect(createBodyOf(paid, GATING)?.elections?.[0].trigger_key).toBe('paid');
	});

	it('refuses a gating marketplace with no licence chosen, naming it', () => {
		const bare = { ...complete(), inventories: ['Tes' as const], free: true, licence: null };
		expect(unlicensed(bare, GATING)).toEqual(['Tes']);
		// One sentence in the panel that asks for it, rather than one in a list
		// at the foot of the page.
		const said = refusalsIn(refusalsOf(bare, VOCABULARY, GATING), 'tes_options');
		expect(said).toHaveLength(1);
		expect(said[0].message).toBe('Choose a licence. Tes needs one to list this.');
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
		return mark.repeat(64);
	}

	it('starts with four empty slots', () => {
		expect(emptySlots()).toHaveLength(4);
		expect(thumbnailHashes(emptySlots())).toEqual([]);
	});

	it('carries only the slots the upload answered for, in slot order', () => {
		const slots = emptySlots();
		slots[0] = { local: 'blob:a', handle: handle('a'), sending: false, refusal: null };
		// Chosen and drawn, but still in flight: the picture is on the page and
		// the handle is not, and only the handle may reach the wire.
		slots[1] = { local: 'blob:b', handle: null, sending: true, refusal: null };
		slots[2] = { local: 'blob:c', handle: handle('c'), sending: false, refusal: null };
		expect(thumbnailHashes(slots).map((hash) => hash[0])).toEqual(['a', 'c']);
	});

	it('seeds a slot per stored digest and leaves the rest empty', () => {
		const seeded = slotsFrom([handle('a'), handle('c')]);
		expect(seeded).toHaveLength(4);
		expect(thumbnailHashes(seeded)).toEqual([handle('a'), handle('c')]);
		// Drawn from the blob's own route, so the seller sees the picture that
		// is stored rather than a caption standing in for one.
		expect(seeded[0].local).toBe(`/v1/uploads/${handle('a')}`);
		expect(seeded[2].local).toBeNull();
		expect(slotsSettling(seeded)).toBe(false);
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

describe('what the seller is told once the listing exists', () => {
	// Found live: an empty mapping list fell through to the plural arm, so a
	// resource kept here was announced as being on "0 marketplaces".

	it('says only that it was created when no marketplace was chosen', () => {
		expect(createdToast(0)).toBe('Listing created.');
		expect(createdToast(0)).not.toContain('0 marketplaces');
	});

	it('counts one marketplace as a word and several as a figure', () => {
		expect(createdToast(1)).toContain('one marketplace');
		expect(createdToast(3)).toContain('3 marketplaces');
	});

	it('counts the marketplaces the teacher ticked', () => {
		expect(marketplacesReached(['Tpt', 'Tes'])).toBe(2);
		expect(marketplacesReached([])).toBe(0);
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
		expect(shouldLandOnCreated('/resources/new', '/resources/new')).toBe(true);
	});

	it('leaves the seller where they went when they navigated away mid-create', () => {
		expect(shouldLandOnCreated('/resources/new', '/analytics')).toBe(false);
		expect(shouldLandOnCreated('/resources/new', '/resources')).toBe(false);
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

describe('the edit form, seeded from what is stored', () => {
	// The create form and the edit form are one component in two modes, which
	// only works if the seed is genuinely the inverse of the body. These are the
	// assertions that fail when a field is added to one direction and not the
	// other; without them the seventeen sidecar fields go back to being set once
	// and never changed, silently.

	/** The read as the server composes it from a body the create sent.
	 *
	 *  Hand-written rather than captured, and that is this file's own exposure:
	 *  it mirrors `ProductView` in `crates/tam-api/src/resources.rs` and nothing
	 *  compares the two, so a field renamed there leaves this agreeing with
	 *  itself. The `tpt_base` block is `TptBaseView`, which is deliberately the
	 *  same shape `TptBaseInput` reads. */
	function viewOf(body: CreateProductBody): ProductView {
		const base = body.tpt_base;
		return {
			id: 'c0ffee00-0000-4000-8000-000000000001',
			title: body.title,
			body: body.body ?? '',
			body_format: body.body_format ?? 'Markdown',
			price: body.price,
			files: [
				...(body.payload ?? []).map((file, index) => ({
					id: `payload-${index}`,
					role: 'payload' as const,
					kind: file.kind,
					byte_len: file.byte_len,
					hash: file.hash,
					scan: 'clean',
					name: file.name
				})),
				...(body.cover === null || body.cover === undefined
					? []
					: [
							{
								id: 'cover-0',
								role: 'cover' as const,
								kind: body.cover.kind,
								byte_len: body.cover.byte_len,
								hash: body.cover.hash,
								scan: 'clean'
							}
						]),
				...(body.previews ?? []).map((file, index) => ({
					id: `preview-${index}`,
					role: 'preview' as const,
					kind: file.kind,
					byte_len: file.byte_len,
					hash: file.hash,
					scan: 'clean',
					name: file.name
				}))
			],
			subjects: body.subjects ?? [],
			grades: {
				source: 'seller',
				raw: (body.grades ?? []).map((path) => ({
					inventory: path.inventory,
					kind: 'phase' as const,
					segments: path.segments,
					native_id: path.native_id ?? undefined
				}))
			},
			rights:
				body.rights === null || body.rights === undefined
					? undefined
					: {
							inventory: body.rights.inventory,
							kind: 'licence' as const,
							segments: body.rights.segments,
							native_id: body.rights.native_id ?? undefined
						},
			tpt_base:
				base === undefined
					? undefined
					: {
							thumbnail_mode: base.thumbnail_mode ?? 1,
							thumbnail_hashes: base.thumbnail_hashes ?? [],
							video_preview_hash: base.video_preview_hash ?? null,
							additional_licence_minor_units: base.additional_licence_minor_units ?? null,
							bundle_discount_minor_units: base.bundle_discount_minor_units ?? null,
							tax_code_id: base.tax_code_id ?? null,
							subject_areas: base.subject_areas ?? [],
							tags: base.tags ?? [],
							formats: base.formats ?? [],
							custom_categories: base.custom_categories ?? [],
							appropriate_for_country: base.appropriate_for_country ?? null,
							standards: (base.standards ?? []).map((pick) => ({
								framework: pick.framework,
								code: pick.code,
								tpt_node_id: pick.tpt_node_id ?? null
							})),
							teaching_duration_id: base.teaching_duration_id ?? null,
							pages_or_slides: base.pages_or_slides ?? null,
							answer_key_id: base.answer_key_id ?? null,
							copyright_declaration_id: base.copyright_declaration_id ?? null,
							status_user: base.status_user ?? 0
						},
			created_at: 1,
			updated_at: 2
		};
	}

	/** Everything the two bodies carry, filled with a distinct value each, so a
	 *  field lost in either direction shows up as a difference rather than as
	 *  two defaults agreeing. */
	function filled(): TptDraft {
		return {
			...complete(),
			description: 'Twelve tasks on a number line.',
			free: false,
			price: '4.50',
			additionalLicence: '3.15',
			bundleDiscount: '2.00',
			taxCode: '1',
			thumbnailMode: '2',
			thumbnails: ['b'.repeat(64), 'c'.repeat(64)],
			grades: ['3rd-grade', '4th-grade'],
			subjectAreas: ['math'],
			tags: ['centers'],
			formats: ['pdf'],
			customCategories: ['Autumn term'],
			appropriateForCountry: true,
			standards: [
				{
					framework: 1,
					code: '3.NF.A.2',
					statement: 'Understand a fraction as a number on the number line.',
					source_guid: 'ccss-3nfa2',
					tpt_node_id: 4211
				}
			],
			teachingDuration: '3',
			pagesOrSlides: '12',
			answerKey: '1',
			copyright: '1',
			status: '1'
		};
	}

	it('fills every sidecar field from a product that carries one', () => {
		const seeded = draftOf(viewOf(createBodyOf(filled()) as CreateProductBody), ['Tpt']);
		expect(seeded.thumbnailMode).toBe('2');
		expect(seeded.thumbnails).toEqual(['b'.repeat(64), 'c'.repeat(64)]);
		expect(seeded.additionalLicence).toBe('3.15');
		expect(seeded.bundleDiscount).toBe('2.00');
		expect(seeded.taxCode).toBe('1');
		expect(seeded.subjectAreas).toEqual(['math']);
		expect(seeded.tags).toEqual(['centers']);
		expect(seeded.formats).toEqual(['pdf']);
		expect(seeded.customCategories).toEqual(['Autumn term']);
		expect(seeded.appropriateForCountry).toBe(true);
		expect(seeded.standards.map((pick) => pick.code)).toEqual(['3.NF.A.2']);
		expect(seeded.teachingDuration).toBe('3');
		expect(seeded.pagesOrSlides).toBe('12');
		expect(seeded.answerKey).toBe('1');
		expect(seeded.copyright).toBe('1');
		expect(seeded.status).toBe('1');
		expect(seeded.inventories).toEqual(['Tpt']);
	});

	it('leaves the attestation and the tax code unstated where no sidecar exists', () => {
		const bare = viewOf(createBodyOf(complete()) as CreateProductBody);
		const seeded = draftOf({ ...bare, tpt_base: undefined });
		// Neither is ours to invent: TPT's own form arrives with the first
		// attestation ticked and ours must not, and designating a tax code is
		// the seller's under TPT's terms (D7).
		expect(seeded.copyright).toBeNull();
		expect(seeded.taxCode).toBeNull();
		expect(seeded.thumbnailMode).toBe(emptyTptDraft().thumbnailMode);
		expect(seeded.status).toBe(emptyTptDraft().status);
		expect(seeded.subjectAreas).toEqual([]);
	});

	it('round-trips a filled draft through the create body and the read', () => {
		const original = filled();
		const seeded = draftOf(viewOf(createBodyOf(original) as CreateProductBody), original.inventories);
		// Every field either body carries. `overrides` is neither body's and
		// `standards` is compared on what the sidecar stores: the mirror's own
		// identifier and the published statement are not columns, so the seed
		// synthesises a key and says so.
		const { standards: seededStandards, ...seededRest } = seeded;
		const { standards: originalStandards, ...originalRest } = original;
		expect(seededRest).toEqual(originalRest);
		expect(seededStandards.map(({ framework, code, tpt_node_id }) => ({
			framework,
			code,
			tpt_node_id
		}))).toEqual(
			originalStandards.map(({ framework, code, tpt_node_id }) => ({
				framework,
				code,
				tpt_node_id
			}))
		);
	});

	it('carries the format the stored body is written in, rather than asserting one', () => {
		// The Tes import writes `Html` bodies, and an edit that relabelled one
		// Markdown would leave the markup unchanged under a declaration that is
		// now wrong: the next send renders any `*`, `_`, `#` or leading `- ` in
		// the seller's prose as markup they did not write.
		const stored = viewOf(createBodyOf(filled()) as CreateProductBody);
		const html: ProductView = {
			...stored,
			body: '<p>Ten pages of practice.</p>',
			body_format: 'Html'
		};
		const seeded = draftOf(html);
		expect(seeded.bodyFormat).toBe('Html');
		expect(patchBodyOf(seeded)?.body_format).toBe('Html');
		expect(patchBodyOf(seeded)?.body).toBe('<p>Ten pages of practice.</p>');
	});

	it('writes Markdown on the create path, where no control offers the other', () => {
		expect(emptyTptDraft().bodyFormat).toBe('Markdown');
		expect(createBodyOf(filled())?.body_format).toBe('Markdown');
	});

	it('leaves an unanswered localisation question unanswered', () => {
		// The sidecar holds three states and the server has its own test saying
		// so. Reading `null` as `false` would turn "never asked" into
		// "explicitly no" on the first save, because the block travels whole.
		const stored = viewOf(createBodyOf(filled()) as CreateProductBody);
		const unasked: ProductView = {
			...stored,
			tpt_base: { ...(stored.tpt_base as TptBaseView), appropriate_for_country: null }
		};
		const seeded = draftOf(unasked);
		expect(seeded.appropriateForCountry).toBeNull();
		expect(tptBaseOf(seeded).appropriate_for_country).toBeNull();
		expect(patchBodyOf(seeded)?.tpt_base?.appropriate_for_country).toBeNull();
		expect(draftInputOf(seeded).appropriate_for_country).toBeNull();
		// And the null survives the wasm boundary. The core's own field is
		// `Option<bool>`, so it should; this is here because a value that
		// crossed as a boolean until now is exactly the kind that fails on the
		// first null and takes the whole refusal list with it.
		expect(refusalsOf(seeded, VOCABULARY)).toEqual([]);
	});

	it('keeps a ticked and an unticked answer apart from an unanswered one', () => {
		const stored = viewOf(createBodyOf(filled()) as CreateProductBody);
		const base = stored.tpt_base as TptBaseView;
		const answered = (value: boolean | null) =>
			draftOf({ ...stored, tpt_base: { ...base, appropriate_for_country: value } })
				.appropriateForCountry;
		expect(answered(true)).toBe(true);
		expect(answered(false)).toBe(false);
		expect(answered(null)).toBeNull();
	});

	it('starts a create at unticked, which is an answer because the control is shown', () => {
		expect(emptyTptDraft().appropriateForCountry).toBe(false);
	});

	it('carries the sidecar whole and never merges it', () => {
		const body = patchBodyOf(filled());
		expect(body?.tpt_base).toEqual(tptBaseOf(filled()));
	});

	it('sends the body format only alongside the body it describes', () => {
		const body = patchBodyOf(filled());
		// The server refuses a format on its own, and a format that moved
		// without its text is how a Markdown listing acquires escaped markup.
		expect(body?.body_format).toBeDefined();
		expect(body?.body).toBeDefined();
	});

	it('sends neither the files nor the canonical subjects, which it would clear', () => {
		const body = patchBodyOf(filled()) as Record<string, unknown>;
		// An absent field is left as stored on this route, so a form with no
		// control for one must omit it: the create's own empty subject list
		// would wipe a taxonomy an import wrote.
		expect('subjects' in body).toBe(false);
		expect('payload' in body).toBe(false);
		expect('cover' in body).toBe(false);
		expect('previews' in body).toBe(false);
		expect('inventories' in body).toBe(false);
		expect('elections' in body).toBe(false);
	});

	it('refuses to compose a body from a price it will not send', () => {
		expect(patchBodyOf({ ...filled(), free: false, price: 'four fifty' })).toBeNull();
	});
});

describe('taking back an upload before the draft is made', () => {
	it('drops the file and leaves every other field alone', () => {
		const chosen = { ...complete(), thumbnailMode: '2', thumbnails: ['b'.repeat(64)] };
		const dropped = withoutPayload(chosen);
		expect(dropped.payload).toEqual([]);
		expect(dropped.cover).toBeNull();
		expect(dropped.previews).toEqual([]);
		// The four slots are separate uploads with their own handles, and the
		// price, the marketplaces and every sidecar field are untouched.
		expect(dropped.thumbnails).toEqual(['b'.repeat(64)]);
		expect(dropped.inventories).toEqual(chosen.inventories);
		expect(dropped.free).toBe(chosen.free);
		expect(tptBaseOf(dropped)).toEqual(tptBaseOf(chosen));
	});

	it('opens the file-first refusal where a marketplace is already ticked', () => {
		expect(needsFileBeforeMarketplace(withoutPayload(complete()))).toBe(true);
		expect(needsFileBeforeMarketplace(withoutPayload({ ...complete(), inventories: [] }))).toBe(
			false
		);
	});

	it('still composes a create for a resource kept here with no file (D32)', () => {
		const kept = withoutPayload({ ...complete(), inventories: [] });
		const body = createBodyOf(kept);
		expect(body).not.toBeNull();
		expect(body?.payload).toEqual([]);
	});

	it('tells the compiled core the file is gone', () => {
		expect(draftInputOf(withoutPayload(complete())).payload_hash).toBeNull();
	});
});

describe('one tile, one marketplace, one inventory', () => {
	// The founder's rule of 2026-09-12: Tes is one marketplace with no
	// regions, so a ticked tile is the whole answer to where a listing goes.

	it('sends the inventory of every tile ticked', () => {
		expect(inventoriesOf(['Tpt'])).toEqual(['Tpt']);
		expect(inventoriesOf(['Tes'])).toEqual(['Tes']);
	});

	it('orders by the tile grid rather than by the order they were ticked', () => {
		expect(inventoriesOf(['Tes', 'Tpt'])).toEqual(['Tpt', 'Tes']);
	});

	it('sends nothing for a marketplace no adapter exists for', () => {
		expect(inventoriesOf(['Etsy'])).toEqual([]);
	});

	// An edit form opens on the listing's own mappings.
	it('reads a stored listing back as the tiles it was composed from', () => {
		expect(tilesOf(['Tpt', 'Tes'])).toEqual({ marketplaces: ['Tpt', 'Tes'] });
		expect(tilesOf([])).toEqual({ marketplaces: [] });
	});

	it('round-trips the tiles a listing was composed from', () => {
		const listing = withMarketplaces(complete(), ['Tpt', 'Tes']);
		expect(tilesOf(listing.inventories).marketplaces).toEqual(listing.marketplaces);
	});
});

describe('the grade grid in either system', () => {
	const grade = (slug: string) =>
		VOCABULARY.grades.find((facet) => facet.slug === slug) as FacetView;

	it('reads the declared British label, which is a table and never an inference', () => {
		expect(gradeLabel(grade('8th-grade'), 'british')).toBe('Year 9');
		expect(gradeLabel(grade('8th-grade'), 'american')).toBe('8th Grade');
	});

	// Preschool, Higher Education, Adult Education and Not Grade Specific have
	// no year group, and inventing one is the inference the founder's table
	// exists to avoid. A blank there would be worse than the American word.
	it('falls back to the American label where no British one is declared', () => {
		expect(grade('preschool').british_label).toBeNull();
		expect(gradeLabel(grade('preschool'), 'british')).toBe('Preschool');
		expect(gradeLabel(grade('not-grade-specific'), 'british')).toBe('Not Grade Specific');
	});

	it('heads each column in the system the teacher chose', () => {
		expect(gradeBandLabel(VOCABULARY, 0, 'american')).toBe('Elementary');
		expect(gradeBandLabel(VOCABULARY, 0, 'british')).toBe('Primary School');
		expect(gradeBandLabel(VOCABULARY, 2, 'british')).toBe('College');
	});

	it('heads nothing where the server headed no such column', () => {
		expect(gradeBandLabel(VOCABULARY, 9, 'american')).toBe('');
	});

	// One selection underneath either way: the slug a teacher ticks is the
	// same slug whichever words it was shown to them in, which is what makes
	// the toggle a relabelling rather than a second field.
	it('changes no selection when the words change', () => {
		const picked = { ...complete(), grades: ['8th-grade'] };
		expect(draftInputOf(picked).grades).toEqual(['8th-grade']);
	});
});

describe('the description formatting bar', () => {
	it('wraps the selection and keeps it selected', () => {
		const marked = markUp('a bold word', 2, 6, 'bold');
		expect(marked.text).toBe('a **bold** word');
		expect(marked.text.slice(marked.start, marked.end)).toBe('bold');
	});

	it('leaves the caret between the marks when nothing is selected', () => {
		const marked = markUp('ab', 1, 1, 'italic');
		expect(marked.text).toBe('a**b');
		expect(marked.start).toBe(2);
		expect(marked.end).toBe(2);
	});

	it('writes a bullet per line the selection touches, from the line start', () => {
		// From the line start rather than from the caret: prefixing mid-line
		// would put the bullet inside a sentence.
		const marked = markUp('one\ntwo', 1, 5, 'bullets');
		expect(marked.text).toBe('- one\n- two');
	});

	it('numbers the lines it marks', () => {
		expect(markUp('one\ntwo\nthree', 0, 13, 'numbers').text).toBe('1. one\n2. two\n3. three');
	});

	it('leaves the rest of the description alone', () => {
		const marked = markUp('before\nmid\nafter', 7, 10, 'bullets');
		expect(marked.text).toBe('before\n- mid\nafter');
	});
});

describe('the files a teacher added', () => {
	function stored(name: string, hashes: string[]): StoredFile {
		return {
			name,
			bytes: 2048,
			payload: hashes.map((hash) => ({ hash, kind: 'pdf' as const, byte_len: 1024 })),
			cover: { hash: `cover-${name}`, kind: 'image' as const, byte_len: 512 },
			storedBytes: 4096,
			storageBytesMax: 8192
		};
	}

	it('joins every file handle in the order they were added', () => {
		const { payload } = payloadOf([stored('a.pdf', ['h1']), stored('b.pdf', ['h2', 'h3'])]);
		expect(payload.map((handle) => handle.hash)).toEqual(['h1', 'h2', 'h3']);
	});

	it('draws the thumbnail from the file buyers see first', () => {
		expect(payloadOf([stored('a.pdf', ['h1']), stored('b.pdf', ['h2'])]).cover?.hash).toBe(
			'cover-a.pdf'
		);
	});

	it('has no thumbnail at all where no file was added', () => {
		expect(payloadOf([])).toEqual({ payload: [], cover: null });
	});
});
