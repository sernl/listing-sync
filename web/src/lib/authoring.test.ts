import { describe, expect, it } from 'vitest';
import {
	axisControls,
	createBodyOf,
	emptyDraft,
	formatBytes,
	licenceElections,
	licenceOptions,
	licenceValues,
	measure,
	payloadRefusal,
	priceOf,
	quotaSentence,
	refusalsOf,
	rightsOf,
	submittable,
	toMinorUnits,
	toggleSubject,
	unansweredRequired,
	type Draft
} from './authoring';
import { AUTHORABLE_PLATFORMS, platformTitle } from './platforms';
import type { VocabularyView } from '$lib/api';
import type { InventoryId } from '$lib/generated/vocab';

/** The Tes GB vocabulary as the endpoint serves it, trimmed to the fields
 *  these tests read. Transcribed from `crates/tam-api/src/vocabulary.rs` and
 *  the Tes registry rather than invented. */
const TES_GB: VocabularyView = {
	inventory: 'TesGb',
	marketplace: 'Tes',
	canonical: [
		{ field: 'title', required: false },
		{ field: 'description', required: false },
		{ field: 'price', required: false },
		{ field: 'taxonomy', required: false },
		{ field: 'grades', required: false },
		{ field: 'files', required: false }
	],
	natives: [
		{
			name: 'licence',
			direction: 'both',
			required: true,
			vocabulary: 'closed',
			values: [
				{ id: 'CC-BY', label: 'Creative Commons Attribution 4.0 International licence' },
				{
					id: 'CC-BY-ND',
					label: 'Creative Commons Attribution-NoDerivatives 4.0 International licence'
				},
				{
					id: 'CC-BY-SA',
					label: 'Creative Commons Attribution-ShareAlike 4.0 International licence'
				},
				{ id: 'TES-PAID', label: 'Teaching Resource Licence' }
			],
			delegation: { kind: 'never', reason: 'legal_content' }
		},
		{
			name: 'mainType',
			direction: 'written',
			required: false,
			vocabulary: 'closed',
			values: [
				{ id: '99001', label: 'Assembly' },
				{ id: '99002', label: 'Assessment and revision' }
			],
			delegation: { kind: 'by_opt_in' }
		},
		{
			name: 'ageRanges',
			direction: 'both',
			required: false,
			vocabulary: 'closed',
			values: [
				{ id: '1', label: '3-5' },
				{ id: '2', label: '5-7' },
				{ id: '3', label: '7-11' },
				{ id: '4', label: '11-14' },
				{ id: '5', label: '14-16' },
				{ id: '6', label: '16+' },
				{ id: '7', label: 'Age not applicable' }
			],
			delegation: { kind: 'by_opt_in' }
		},
		{
			name: 'curriculum',
			direction: 'read_only',
			required: false,
			vocabulary: 'closed',
			values: [{ id: 'English', label: 'English' }],
			delegation: { kind: 'by_opt_in' }
		}
	],
	axes: [
		{
			axis: 'licence',
			native: 'licence',
			cardinality: 'one',
			delegation: { kind: 'never', reason: 'legal_content' },
			required: true
		},
		{
			axis: 'resource_type',
			native: 'mainType',
			cardinality: 'one',
			delegation: { kind: 'by_opt_in' },
			required: false
		},
		{
			axis: 'phase',
			native: 'ageRanges',
			cardinality: 'many',
			delegation: { kind: 'by_opt_in' },
			required: false
		}
	],
	absent_axes: [],
	authoring: {
		payload_files: 'every_payload_file',
		body_wire: 'carries_declared_format',
		body_formats: ['Markdown', 'Html'],
		licence: {
			native: 'licence',
			free: ['CC-BY', 'CC-BY-ND', 'CC-BY-SA'],
			paid: ['TES-PAID']
		}
	}
};

const TPT: VocabularyView = {
	inventory: 'Tpt',
	marketplace: 'Tpt',
	canonical: [
		{ field: 'title', required: false },
		{ field: 'description', cap: { limit: 45_000, unit: 'Utf16CodeUnits' }, required: false },
		{ field: 'price', required: false },
		{ field: 'taxonomy', required: false },
		{ field: 'grades', required: false },
		{ field: 'files', required: false }
	],
	natives: [
		{
			name: 'taxonomyTags',
			direction: 'both',
			required: false,
			vocabulary: 'closed_uncaptured',
			delegation: { kind: 'by_opt_in' }
		}
	],
	axes: [
		{
			axis: 'resource_type',
			native: 'taxonomyTags',
			cardinality: 'many',
			delegation: { kind: 'by_opt_in' },
			required: false
		}
	],
	absent_axes: ['licence'],
	authoring: {
		payload_files: 'exactly_one',
		body_wire: 'renders_to_html',
		body_formats: ['Markdown', 'Html'],
		price_floor_minor_units: 95,
		attestation: { native: 'ItemsProperty.copyright_declaration', held_per_connection: true }
	}
};

const VOCABULARIES = new Map<InventoryId, VocabularyView>([
	['TesGb', TES_GB],
	['Tpt', TPT]
]);

function handle(hash: string) {
	return { hash, kind: 'pdf' as const, byte_len: 10 };
}

function draftWith(patch: Partial<Draft>): Draft {
	return { ...emptyDraft(), ...patch };
}

describe('the multi-file rule', () => {
	it('lets a Tes platform carry every payload file', () => {
		expect(payloadRefusal('every_payload_file', 4)).toBeNull();
	});

	it('refuses more than one file where the create takes exactly one', () => {
		expect(payloadRefusal('exactly_one', 4)).toMatch(/exactly one file/);
		expect(payloadRefusal('exactly_one', 4)).toMatch(/keep the ZIP whole/);
	});

	it('accepts exactly one, and says so distinctly when there are none', () => {
		expect(payloadRefusal('exactly_one', 1)).toBeNull();
		expect(payloadRefusal('exactly_one', 0)).toMatch(/carries none/);
	});

	it('makes a multi-file product TES-only, as a blocking refusal naming TPT', () => {
		const multi = draftWith({
			title: 'A bundle',
			payload: [handle('a'), handle('b')],
			inventories: ['TesGb', 'Tpt'],
			licence: 'CC-BY'
		});
		const refusals = refusalsOf(multi, VOCABULARIES);
		const files = refusals.filter((refusal) => refusal.field === 'files');
		expect(files).toHaveLength(1);
		expect(files[0].message).toMatch(/^TPT \(Teachers Pay Teachers\)/);
		expect(files[0].blocking).toBe(true);
		expect(submittable(refusals)).toBe(false);
	});

	it('leaves the same product submittable once TPT is dropped', () => {
		const tesOnly = draftWith({
			title: 'A bundle',
			payload: [handle('a'), handle('b')],
			inventories: ['TesGb'],
			licence: 'CC-BY'
		});
		expect(submittable(refusalsOf(tesOnly, VOCABULARIES))).toBe(true);
	});
});

describe('the licence, which is the one required field anywhere', () => {
	it('offers the Creative Commons values on the free branch and TES-PAID on the paid one', () => {
		expect(licenceValues(TES_GB.authoring.licence, 'free')).toEqual([
			'CC-BY',
			'CC-BY-ND',
			'CC-BY-SA'
		]);
		expect(licenceValues(TES_GB.authoring.licence, 'paid')).toEqual(['TES-PAID']);
	});

	it('offers nothing where the platform holds no licence field', () => {
		expect(licenceValues(TPT.authoring.licence, 'free')).toEqual([]);
		expect(licenceOptions(TPT, 'free')).toEqual([]);
		expect(licenceOptions(undefined, 'free')).toEqual([]);
	});

	it('names each gate id with the words the captured vocabulary holds for it', () => {
		expect(licenceOptions(TES_GB, 'free')).toEqual([
			{ id: 'CC-BY', label: 'Creative Commons Attribution 4.0 International licence' },
			{
				id: 'CC-BY-ND',
				label: 'Creative Commons Attribution-NoDerivatives 4.0 International licence'
			},
			{
				id: 'CC-BY-SA',
				label: 'Creative Commons Attribution-ShareAlike 4.0 International licence'
			}
		]);
		expect(licenceOptions(TES_GB, 'paid')).toEqual([
			{ id: 'TES-PAID', label: 'Teaching Resource Licence' }
		]);
	});

	it('lets a gate id the capture carries no words for stand in for itself', () => {
		const unlabelled: VocabularyView = {
			...TES_GB,
			natives: TES_GB.natives.map((native) =>
				native.name === 'licence' ? { ...native, values: undefined } : native
			)
		};
		expect(licenceOptions(unlabelled, 'paid')).toEqual([
			{ id: 'TES-PAID', label: 'TES-PAID' }
		]);
	});

	it('reads a Tes selection as unanswered until a licence is chosen', () => {
		const bare = draftWith({ inventories: ['TesGb'] });
		expect(unansweredRequired(bare, VOCABULARIES)).toEqual([
			{ inventory: 'TesGb', native: 'licence' }
		]);
		const answered = draftWith({ inventories: ['TesGb'], licence: 'CC-BY' });
		expect(unansweredRequired(answered, VOCABULARIES)).toEqual([]);
	});

	it('asks nothing of a platform that declares no required field', () => {
		expect(unansweredRequired(draftWith({ inventories: ['Tpt'] }), VOCABULARIES)).toEqual([]);
	});

	it('writes one supply election per licence-gating platform, keyed on the price branch', () => {
		const draft = draftWith({
			inventories: ['TesGb', 'Tpt'],
			licence: 'TES-PAID',
			branch: 'paid'
		});
		expect(licenceElections(draft, VOCABULARIES)).toEqual([
			{
				inventory: 'TesGb',
				axis: 'licence',
				trigger: 'supply',
				trigger_key: 'paid',
				answers: [{ segments: ['TES-PAID'], native_id: 'TES-PAID' }]
			}
		]);
	});

	it('names the grant against a platform that actually holds a licence field', () => {
		const draft = draftWith({ inventories: ['Tpt', 'TesGb'], licence: 'CC-BY' });
		expect(rightsOf(draft, VOCABULARIES)?.inventory).toBe('TesGb');
	});
});

describe('the platform field controls', () => {
	it('drops the licence axis, which the price-gated selector owns', () => {
		expect(axisControls(TES_GB).map((control) => control.axis)).toEqual([
			'resource_type',
			'phase'
		]);
	});

	it('drops a read-only native, which nothing this form writes reaches', () => {
		expect(axisControls(TES_GB).some((control) => control.native === 'curriculum')).toBe(false);
	});

	it('discloses a many-valued axis outside phase rather than collecting a dead answer', () => {
		const tags = axisControls(TPT)[0];
		expect(tags.unwritableReason).toMatch(/carries no field for this axis/);
		expect(tags.values).toBeNull();
		expect(tags.vocabulary).toBe('closed_uncaptured');
	});

	it('carries the words a seller reads beside each platform token', () => {
		const phase = axisControls(TES_GB).find((control) => control.axis === 'phase');
		expect(phase?.values?.map((value) => `${value.id}:${value.label}`)).toEqual([
			'1:3-5',
			'2:5-7',
			'3:7-11',
			'4:11-14',
			'5:14-16',
			'6:16+',
			'7:Age not applicable'
		]);
		const type = axisControls(TES_GB).find((control) => control.axis === 'resource_type');
		expect(type?.values?.[0]).toEqual({ id: '99001', label: 'Assembly' });
	});

	it('states why the licence refuses delegation', () => {
		const licence = TES_GB.natives.find((native) => native.name === 'licence');
		expect(licence?.delegation.reason).toBe('legal_content');
	});
});

describe('the subject picker', () => {
	// Canonical term ids, which are the hyphenated UUIDs `subjects` carries.
	const MATHS = '1b4d2a70-0000-4000-8000-000000000001';
	const ENGLISH = '1b4d2a70-0000-4000-8000-000000000002';

	it('adds a ticked term, in the order the seller ticked it', () => {
		expect(toggleSubject([], MATHS, true)).toEqual([MATHS]);
		expect(toggleSubject([MATHS], ENGLISH, true)).toEqual([MATHS, ENGLISH]);
	});

	it('removes an unticked term and leaves the rest alone', () => {
		expect(toggleSubject([MATHS, ENGLISH], MATHS, false)).toEqual([ENGLISH]);
		expect(toggleSubject([ENGLISH], MATHS, false)).toEqual([ENGLISH]);
	});

	it('writes a term already held once rather than twice', () => {
		expect(toggleSubject([MATHS], MATHS, true)).toEqual([MATHS]);
	});

	it('never mutates the list it was handed', () => {
		const chosen = [MATHS];
		toggleSubject(chosen, ENGLISH, true);
		expect(chosen).toEqual([MATHS]);
	});

	it('carries the chosen ids into the create body’s subjects', () => {
		const draft = draftWith({
			title: 'A worksheet',
			payload: [handle('a')],
			inventories: ['TesGb'],
			licence: 'CC-BY',
			subjects: [MATHS, ENGLISH]
		});
		expect(createBodyOf(draft, VOCABULARIES)?.subjects).toEqual([MATHS, ENGLISH]);
	});

	it('sends an empty list where none was chosen, which is what the server defaults to', () => {
		const draft = draftWith({ title: 'A worksheet', payload: [handle('a')] });
		expect(createBodyOf(draft, VOCABULARIES)?.subjects).toEqual([]);
	});
});

describe('the price', () => {
	it('reads a free draft as the bare Free intent', () => {
		expect(priceOf(draftWith({ branch: 'free' }))).toBe('Free');
	});

	it('scales a typed amount into the denomination’s minor units', () => {
		expect(toMinorUnits('4.50', 'Gbp')).toBe(450);
		expect(toMinorUnits('12', 'Usd')).toBe(1200);
	});

	it('refuses an amount it would have to alter to send', () => {
		expect(toMinorUnits('4.505', 'Gbp')).toBeNull();
		expect(toMinorUnits('', 'Gbp')).toBeNull();
		expect(toMinorUnits('-3', 'Gbp')).toBeNull();
		expect(toMinorUnits('4.50', 'Eur')).toBeNull();
	});

	it('blocks a paid draft with no readable amount, as Money::new would', () => {
		const draft = draftWith({
			title: 'A thing',
			payload: [handle('a')],
			inventories: ['Tpt'],
			branch: 'paid',
			amount: '0'
		});
		const price = refusalsOf(draft, VOCABULARIES).filter((one) => one.field === 'price');
		expect(price[0].blocking).toBe(true);
	});

	it('warns about a floor the create does not enforce but the write does', () => {
		const draft = draftWith({
			title: 'A thing',
			payload: [handle('a')],
			inventories: ['Tpt'],
			branch: 'paid',
			amount: '0.50'
		});
		const floor = refusalsOf(draft, VOCABULARIES).find((one) => one.field === 'price');
		expect(floor?.message).toMatch(/refuses a price below 95 minor units/);
		expect(floor?.blocking).toBe(false);
	});
});

describe('the refusals the form mirrors', () => {
	it('names the payload one with the server’s own remedy', () => {
		const draft = draftWith({ title: 'A thing', inventories: ['TesGb'], licence: 'CC-BY' });
		const files = refusalsOf(draft, VOCABULARIES).find((one) => one.field === 'files');
		expect(files?.message).toMatch(/upload the bytes first/);
		expect(files?.blocking).toBe(true);
	});

	it('refuses a blank title and an empty platform set', () => {
		const fields = refusalsOf(emptyDraft(), VOCABULARIES).map((one) => one.field);
		expect(fields).toContain('title');
		expect(fields).toContain('platforms');
	});

	it('says platforms cannot be added later, because no endpoint adds one', () => {
		const platforms = refusalsOf(emptyDraft(), VOCABULARIES).find(
			(one) => one.field === 'platforms'
		);
		expect(platforms?.message).toMatch(/cannot be added after the draft exists/);
	});

	it('warns about a cap the projection shortens rather than refuses', () => {
		const draft = draftWith({
			title: 'A thing',
			body: 'x'.repeat(45_001),
			payload: [handle('a')],
			inventories: ['Tpt']
		});
		const body = refusalsOf(draft, VOCABULARIES).find((one) => one.field === 'description');
		expect(body?.message).toMatch(/caps the description at 45000/);
		expect(body?.blocking).toBe(false);
		expect(submittable(refusalsOf(draft, VOCABULARIES))).toBe(true);
	});

	it('measures a cap in the unit the cap declares', () => {
		expect(measure('é', 'Bytes')).toBe(2);
		expect(measure('é', 'Utf16CodeUnits')).toBe(1);
		expect(measure('😀', 'Utf16CodeUnits')).toBe(2);
		expect(measure('😀', 'Codepoints')).toBe(1);
		expect(measure('😀', 'GraphemeClusters')).toBe(1);
	});
});

describe('the request the draft composes', () => {
	it('carries the payload, the platforms, the grades and both election kinds', () => {
		const draft = draftWith({
			title: '  A worksheet  ',
			body: '# Heading',
			bodyFormat: 'Markdown',
			payload: [handle('a')],
			inventories: ['TesGb'],
			licence: 'CC-BY',
			axes: { 'TesGb:ageRanges': ['2', '3'], 'TesGb:mainType': ['99002'] }
		});
		const body = createBodyOf(draft, VOCABULARIES);
		expect(body?.title).toBe('A worksheet');
		expect(body?.inventories).toEqual(['TesGb']);
		expect(body?.grades).toEqual([
			{ inventory: 'TesGb', kind: 'phase', segments: ['2'], native_id: '2' },
			{ inventory: 'TesGb', kind: 'phase', segments: ['3'], native_id: '3' }
		]);
		expect(body?.elections).toEqual([
			{
				inventory: 'TesGb',
				axis: 'licence',
				trigger: 'supply',
				trigger_key: 'free',
				answers: [{ segments: ['CC-BY'], native_id: 'CC-BY' }]
			},
			{
				inventory: 'TesGb',
				axis: 'resource_type',
				trigger: 'elect_one',
				answers: [{ segments: ['99002'], native_id: '99002' }]
			}
		]);
	});

	it('composes nothing from a price it would have to alter', () => {
		const draft = draftWith({ branch: 'paid', amount: 'free', payload: [handle('a')] });
		expect(createBodyOf(draft, VOCABULARIES)).toBeNull();
	});
});

describe('the quota refusal', () => {
	it('renders the storage sentence the detail was composed to allow', () => {
		expect(quotaSentence({ quota: 'storage_bytes_max', used: 1024, limit: 2048 })).toBe(
			'Your plan stores up to 2.0 KB and 1.0 KB is in use.'
		);
	});

	it('renders the listing sentence', () => {
		expect(quotaSentence({ quota: 'listings_max', used: 100, limit: 100 })).toMatch(
			/up to 100 listings and 100 are/
		);
	});

	it('falls back to the server’s own message for a detail it cannot read', () => {
		expect(quotaSentence({ quota: 'something_new', used: 1, limit: 2 })).toBeNull();
		expect(quotaSentence('nope')).toBeNull();
	});

	it('writes byte counts in the powers the quota is declared in', () => {
		expect(formatBytes(0)).toBe('0 B');
		expect(formatBytes(1 << 30)).toBe('1.0 GB');
		expect(formatBytes(-1)).toBe('—');
	});
});
