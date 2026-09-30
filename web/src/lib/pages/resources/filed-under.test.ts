import { describe, expect, it } from 'vitest';
import type { ConnectionView, LabelView, MappingHead } from '$lib/api';
import { chipFor } from '$lib/inventory';
import type { InventoryId } from '$lib/generated/vocab';
import { filedUnder } from './filed-under';

const PRODUCT = 'p-fractions';

function mapping(inventory: InventoryId, partial: Partial<MappingHead> = {}): MappingHead {
	return {
		id: `m-${inventory}`,
		product: PRODUCT,
		inventory,
		binding_state: 'bound',
		lifecycle_state: 'live',
		updated_at: 1_788_000_000_000,
		listing_url: null,
		...partial
	};
}

function connection(marketplace: 'Tpt' | 'Tes'): ConnectionView {
	return {
		id: `c-${marketplace}`,
		marketplace,
		state: 'linked',
		status: 'connected',
		created_at: 1,
		updated_at: 2
	};
}

/** The band as the page builds it: the strip through the same `chipFor`,
 *  one chip per marketplace, and the mappings it was built from. */
function band(
	mapped: readonly MappingHead[],
	connections: readonly ConnectionView[],
	labels: readonly LabelView[]
) {
	const chips = (['Tpt', 'Tes', 'Etsy'] as const).map((inventory) =>
		chipFor({
			product: PRODUCT,
			inventory,
			mapping: mapped.find((one) => one.inventory === inventory),
			work: undefined,
			connection: connections.find((one) => one.marketplace === inventory),
			status: undefined
		})
	);
	return filedUnder(mapped, chips, labels);
}

const TPT_MARK: LabelView = { name: 'TPT', colour: 'teal', system: true };
const OWN: LabelView = { name: 'Year 4', colour: 'amber', system: false };
const BOTH = [connection('Tpt'), connection('Tes')];

describe('filedUnder', () => {
	it('names every marketplace the resource is mapped to, with how it stands there', () => {
		const filed = band([mapping('Tpt'), mapping('Tes', { lifecycle_state: 'draft' })], BOTH, [
			TPT_MARK,
			OWN
		]);
		expect(filed.marketplaces.map((one) => [one.name, one.state, one.tone])).toEqual([
			['TPT', 'Listed', 'ok'],
			['Tes', 'Draft', 'mut']
		]);
		expect(filed.labels, "the import's TPT mark is the TPT chip already").toEqual([OWN]);
	});

	it('keeps a marketplace the seller picked but never sent, which the strip calls not listed', () => {
		const filed = band(
			[mapping('Tpt'), mapping('Tes', { binding_state: 'unbound', lifecycle_state: 'draft' })],
			BOTH,
			[TPT_MARK]
		);
		expect(filed.marketplaces.map((one) => [one.name, one.state])).toEqual([
			['TPT', 'Listed'],
			['Tes', 'Not sent yet']
		]);
	});

	it('says a marketplace with no sign-in needs one, in words rather than as a button', () => {
		const filed = band([mapping('Tpt'), mapping('Tes')], [connection('Tpt')], []);
		const tes = filed.marketplaces.find((one) => one.inventory === 'Tes');
		expect([tes?.state, tes?.tone]).toEqual(['Sign-in needed', 'bad']);
	});

	it("keeps an import's mark where no mapping names that shop", () => {
		const filed = band([], BOTH, [TPT_MARK, OWN]);
		expect(filed.marketplaces).toEqual([]);
		expect(filed.labels).toEqual([TPT_MARK, OWN]);
	});
});
