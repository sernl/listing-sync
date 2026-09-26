import { describe, expect, it } from 'vitest';
import {
	formatBytes,
	licenceElections,
	licenceGated,
	licenceOptions,
	licenceValues,
	measure,
	missingFields,
	payloadRefusal,
	priceOf,
	quotaSentence,
	requiredFieldSentence,
	rightsOf,
	toMinorUnits,
	type LicenceIntent
} from './authoring';
import { AUTHORABLE_PLATFORMS, platformTitle } from './platforms';
import type { VocabularyView } from '$lib/api';
import type { InventoryId } from '$lib/generated/vocab';

/** The Tes GB vocabulary as the endpoint serves it, trimmed to the fields
 *  these tests read. Transcribed from `crates/tam-api/src/vocabulary.rs` and
 *  the Tes registry rather than invented. */
const TES_GB: VocabularyView = {
	inventory: 'Tes',
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
	['Tes', TES_GB],
	['Tpt', TPT]
]);

function handle(hash: string) {
	return { hash, kind: 'pdf' as const, byte_len: 10 };
}

/** A draft as these tests build one.
 *
 *  The production `Draft` type is gone: it belonged to a second create path no
 *  page ever rendered, which still carried the marketplace rule the founder had
 *  removed. The functions it used to feed — the payload rule, the licence
 *  helpers, the price — are live and reached from the TPT form through narrower
 *  parameter types, so the fixture shape lives here rather than being exported
 *  for one consumer. */
interface DraftFixture extends LicenceIntent {
	amount: string;
	currency: string;
	payload: { hash: string; kind: 'pdf'; byte_len: number }[];
	title: string;
}

function draftWith(patch: Partial<DraftFixture>): DraftFixture {
	return {
		title: '',
		branch: 'free',
		amount: '',
		currency: 'Gbp',
		payload: [],
		inventories: [],
		licence: null,
		...patch
	};
}

describe('the multi-file rule', () => {
	it('lets a Tes platform carry every payload file', () => {
		expect(payloadRefusal('every_payload_file', 4)).toBeNull();
	});

	it('refuses more than one file where the create takes exactly one', () => {
		expect(payloadRefusal('exactly_one', 4)).toMatch(/exactly one file/);
		expect(payloadRefusal('exactly_one', 4)).toMatch(/keep the ZIP as one file/);
	});

	it('accepts exactly one, and says so distinctly when there are none', () => {
		expect(payloadRefusal('exactly_one', 1)).toBeNull();
		expect(payloadRefusal('exactly_one', 0)).toMatch(/has none/);
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

	it('writes one supply election per licence-gating platform, keyed on the price branch', () => {
		const draft = draftWith({
			inventories: ['Tes', 'Tpt'],
			licence: 'TES-PAID',
			branch: 'paid'
		});
		expect(licenceElections(draft, VOCABULARIES)).toEqual([
			{
				inventory: 'Tes',
				axis: 'licence',
				trigger: 'supply',
				trigger_key: 'paid',
				answers: [{ segments: ['TES-PAID'], native_id: 'TES-PAID' }]
			}
		]);
	});

	it('names the grant against a platform that actually holds a licence field', () => {
		const draft = draftWith({ inventories: ['Tpt', 'Tes'], licence: 'CC-BY' });
		expect(rightsOf(draft, VOCABULARIES)?.inventory).toBe('Tes');
	});

	it('states why the licence refuses delegation', () => {
		const licence = TES_GB.natives.find((native) => native.name === 'licence');
		expect(licence?.delegation.reason).toBe('legal_content');
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

});

describe('the quota refusal', () => {
	it('renders the storage sentence the detail was composed to allow', () => {
		expect(quotaSentence({ quota: 'storage_bytes_max', used: 1024, limit: 2048 })).toBe(
			'Your plan holds up to 2.0 KB of files, and you are using 1.0 KB.'
		);
	});

	it('renders the catalogue sentence in the word the console uses for one', () => {
		expect(quotaSentence({ quota: 'listings_max', used: 100, limit: 100 })).toMatch(
			/up to 100 resources, and you have 100/
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

describe('the refusal that a marketplace wants a field this listing lacks', () => {
	// The server has always carried the identity: `required_fields_answered`
	// in `crates/tam-api/src/catalogue.rs` composes `detail.missing` as
	// `{inventory, field}` entries. The client read none of it, so a seller
	// creating for Tes was told only that "a selected platform requires a
	// field this product does not carry".

	it('names the marketplace and the field in words a seller reads', () => {
		expect(requiredFieldSentence({ missing: [{ inventory: 'Tes', field: 'licence' }] })).toBe(
			`${platformTitle('Tes')} needs a licence, and this listing does not have one yet.`
		);
	});

	it('names every marketplace that asked, rather than only the first', () => {
		const said = requiredFieldSentence({
			missing: [
				{ inventory: 'Tes', field: 'licence' },
				{ inventory: 'Tpt', field: 'licence' }
			]
		});
		expect(said).toContain(platformTitle('Tes'));
		expect(said).toContain(platformTitle('Tpt'));
	});

	it('reads the server’s own label for the field where it sent one', () => {
		expect(
			requiredFieldSentence({
				missing: [{ inventory: 'Tes', field: 'licence', label: 'Licence' }]
			})
		).toContain('needs a licence');
	});

	it('stands a field the registry adds later in for itself rather than guessing', () => {
		expect(requiredFieldSentence({ missing: [{ inventory: 'Tes', field: 'age_range' }] })).toContain(
			'needs a age_range'
		);
	});

	it('falls back to the server’s own sentence for a detail it cannot read', () => {
		expect(requiredFieldSentence({ missing: 'licence' })).toBeNull();
		expect(requiredFieldSentence(null)).toBeNull();
		expect(missingFields({ missing: [{ inventory: 'Tes' }] })).toEqual([]);
	});
});

describe('a marketplace that carries no licence field', () => {
	// The wire omits the key — `licence` carries `skip_serializing_if =
	// "Option::is_none"` — but a JSON source that writes `"licence": null`
	// instead used to pass an `=== undefined` guard and throw one line later on
	// `gate.native`. The whole create form went down with it: the tick was
	// checked, the effect that read the vocabulary threw, and the draft never
	// recorded the marketplace, so the seller saw a ticked box beside a counter
	// reading "kept here". Absent and null are read the same way now.

	function withLicence(value: unknown): VocabularyView {
		return {
			inventory: 'Tpt',
			marketplace: 'Tpt',
			canonical: [],
			natives: [],
			axes: [],
			absent_axes: [],
			authoring: {
				payload_files: 'exactly_one',
				body_wire: 'renders_to_html',
				body_formats: ['Markdown'],
				licence: value
			}
		} as unknown as VocabularyView;
	}

	it('is not gated when the key is absent', () => {
		const absent = withLicence(undefined);
		expect(licenceGated(['Tpt'], new Map([['Tpt', absent]]))).toEqual([]);
		expect(licenceOptions(absent, 'free')).toEqual([]);
	});

	it('is not gated when the key is written out as null', () => {
		const written = withLicence(null);
		expect(licenceGated(['Tpt'], new Map([['Tpt', written]]))).toEqual([]);
		expect(() => licenceOptions(written, 'free')).not.toThrow();
		expect(licenceOptions(written, 'free')).toEqual([]);
	});
});

describe('measuring a value against a cap', () => {
	// `unit` comes off the wire and the index had no guard, so a unit added to
	// the registry and served before this client is rebuilt threw inside a
	// render — which blanks the whole page rather than the counter.

	it('counts in each unit this client knows', () => {
		expect(measure('abc', 'Utf16CodeUnits')).toBe(3);
		expect(measure('é', 'Bytes')).toBe(2);
		expect(measure('é', 'Codepoints')).toBe(1);
	});

	it('falls back rather than throwing on a unit it has never seen', () => {
		const unknown = 'Furlongs' as unknown as Parameters<typeof measure>[1];
		expect(() => measure('abc', unknown)).not.toThrow();
		expect(measure('abc', unknown)).toBe(3);
	});
});
