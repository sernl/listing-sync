import { describe, expect, it } from 'vitest';
import {
	connectionFor,
	loweringRefusal,
	readinessOf,
	uncapturedTransition,
	type ReadinessInput
} from './publish-readiness';
import type { ConnectionView, MappingHead, VocabularyView } from '$lib/api';
import type { InventoryId } from '$lib/generated/vocab';

function mapping(patch: Partial<MappingHead> = {}): MappingHead {
	return {
		id: 'm1',
		product: 'p1',
		inventory: 'Tes',
		binding_state: 'unbound',
		lifecycle_state: 'absent',
		updated_at: 0,
		listing_url: null,
		...patch
	};
}

function connection(patch: Partial<ConnectionView> = {}): ConnectionView {
	return {
		id: 'c1',
		marketplace: 'Tes',
		state: 'linked',
		status: 'connected',
		created_at: 0,
		updated_at: 0,
		...patch
	};
}

function vocabulary(inventory: InventoryId, patch: Partial<VocabularyView> = {}): VocabularyView {
	const tes = inventory !== 'Tpt';
	return {
		inventory,
		marketplace: tes ? 'Tes' : 'Tpt',
		canonical: [],
		natives: tes
			? [
					{
						name: 'licence',
						direction: 'both',
						required: true,
						vocabulary: 'closed',
						values: [
						{
							id: 'CC-BY',
							label: 'Creative Commons Attribution 4.0 International licence'
						}
					],
						delegation: { kind: 'never', reason: 'legal_content' }
					}
				]
			: [],
		axes: tes
			? [
					{
						axis: 'licence',
						native: 'licence',
						cardinality: 'one',
						delegation: { kind: 'never', reason: 'legal_content' },
						required: true
					}
				]
			: [],
		absent_axes: tes ? [] : ['licence'],
		authoring: {
			payload_files: tes ? 'every_payload_file' : 'exactly_one',
			body_wire: tes ? 'carries_declared_format' : 'renders_to_html',
			body_formats: ['Markdown', 'Html']
		},
		...patch
	};
}

function input(patch: Partial<ReadinessInput> = {}): ReadinessInput {
	return {
		inventory: 'Tes',
		intent: 'draft',
		mapping: mapping(),
		connection: connection(),
		status: { inventory: 'Tes', marketplace: 'Tes', halted: false },
		vocabulary: vocabulary('Tes'),
		payloadFiles: 1,
		hasRights: true,
		...patch
	};
}

describe('the transitions no capture supports', () => {
	it('refuses both Tes transitions out of live and names the capability', () => {
		expect(uncapturedTransition('Tes', 'live', 'live')).toBe('tes.edit_published');
		expect(uncapturedTransition('Tes', 'live', 'draft')).toBe('tes.unpublish');
	});

	it('serves both transitions out of draft on Tes', () => {
		expect(uncapturedTransition('Tes', 'draft', 'live')).toBeNull();
		expect(uncapturedTransition('Tes', 'draft', 'draft')).toBeNull();
	});

	it('serves all four on TPT', () => {
		for (const from of ['draft', 'live'] as const) {
			for (const to of ['draft', 'live'] as const) {
				expect(uncapturedTransition('Tpt', from, to)).toBeNull();
			}
		}
	});
});

describe('the lowering, mirrored', () => {
	it('lowers an unbound or severed mapping to either state', () => {
		for (const binding_state of ['unbound', 'severed']) {
			expect(loweringRefusal(mapping({ binding_state }), 'live')).toBeNull();
		}
	});

	it('refuses a binding state that means a write is already in flight', () => {
		expect(loweringRefusal(mapping({ binding_state: 'binding' }), 'draft')).toMatch(
			/already in flight/
		);
	});

	it('refuses a bound mapping whose lifecycle nobody has recorded', () => {
		expect(
			loweringRefusal(mapping({ binding_state: 'bound', lifecycle_state: 'absent' }), 'draft')
		).toMatch(/have not recorded/);
	});

	it('refuses editing a Tes listing that is already live', () => {
		const live = mapping({ binding_state: 'bound', lifecycle_state: 'live' });
		expect(loweringRefusal(live, 'live')).toMatch(/already live here/);
		expect(loweringRefusal(live, 'draft')).toMatch(/back to draft is uncaptured/);
	});

	it('lets the same live listing be revised on TPT', () => {
		const live = mapping({ inventory: 'Tpt', binding_state: 'bound', lifecycle_state: 'live' });
		expect(loweringRefusal(live, 'live')).toBeNull();
	});
});

describe('the readiness line', () => {
	it('reads ready when nothing this client holds refuses the send', () => {
		const verdict = readinessOf(input());
		expect(verdict.ready).toBe(true);
		expect(verdict.line).toBe('ready to send');
		expect(verdict.title).toBe('TES (Tes.com)');
	});

	it('waits rather than claiming ready before the vocabulary is known', () => {
		const verdict = readinessOf(input({ vocabulary: undefined }));
		expect(verdict.ready).toBe(false);
		expect(verdict.tone).toBe('mut');
	});

	it('names the missing connection before anything else it could also say', () => {
		const verdict = readinessOf(
			input({ connection: undefined, hasRights: false, payloadFiles: 0 })
		);
		expect(verdict.line).toMatch(/no account is linked/);
	});

	it('names a disconnected link with the action that fixes it', () => {
		const verdict = readinessOf(input({ connection: connection({ status: 'disconnected' }) }));
		expect(verdict.line).toMatch(/re-link it first/);
	});

	it('reports a halt with the reason the status endpoint recorded', () => {
		const verdict = readinessOf(
			input({
				status: { inventory: 'Tes', marketplace: 'Tes', halted: true, reason: 'upstream outage' }
			})
		);
		expect(verdict.line).toBe('sending is paused for this platform: upstream outage');
		expect(verdict.tone).toBe('run');
	});

	it('needs a licence where the registry declares one required and none is carried', () => {
		const verdict = readinessOf(input({ hasRights: false }));
		expect(verdict.line).toBe('needs a licence');
		expect(verdict.ready).toBe(false);
	});

	it('asks TPT for no licence at all, because it holds no licence field', () => {
		const verdict = readinessOf(
			input({
				inventory: 'Tpt',
				mapping: mapping({ inventory: 'Tpt' }),
				connection: connection({ marketplace: 'Tpt' }),
				status: undefined,
				vocabulary: vocabulary('Tpt'),
				hasRights: false
			})
		);
		expect(verdict.ready).toBe(true);
	});

	it('refuses TPT for a multi-file product, which is the TES-only rule', () => {
		const verdict = readinessOf(
			input({
				inventory: 'Tpt',
				mapping: mapping({ inventory: 'Tpt' }),
				connection: connection({ marketplace: 'Tpt' }),
				status: undefined,
				vocabulary: vocabulary('Tpt'),
				hasRights: false,
				payloadFiles: 3
			})
		);
		expect(verdict.line).toBe('takes exactly one file, and this product carries 3');
		expect(verdict.tone).toBe('bad');
	});

	it('says a platform is not one of this product’s when no mapping carries it', () => {
		const verdict = readinessOf(input({ mapping: undefined }));
		expect(verdict.line).toMatch(/chosen when the draft is created/);
	});

	it('warns about an unstable link only once nothing else stands in the way', () => {
		const verdict = readinessOf(input({ connection: connection({ status: 'unstable' }) }));
		expect(verdict.ready).toBe(false);
		expect(verdict.tone).toBe('run');
		expect(verdict.line).toMatch(/failing verification/);
	});
});

describe('which connection carries a platform', () => {
	it('matches on the marketplace, because a link is held per marketplace', () => {
		const links = [connection({ id: 'tes' }), connection({ id: 'tpt', marketplace: 'Tpt' })];
		expect(connectionFor('Tes', links)?.id).toBe('tes');
		expect(connectionFor('Tpt', links)?.id).toBe('tpt');
		expect(connectionFor('Etsy', links)).toBeUndefined();
	});
});
