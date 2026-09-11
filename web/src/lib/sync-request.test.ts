import { describe, expect, it } from 'vitest';
import {
	AUTHORSHIP_FIRST,
	FILES_STAY_ON_YOUR_COMPUTER,
	NOT_AN_IMPORT,
	TERM_COVERAGE_LEGEND,
	WAITING_IN_LIST,
	MIGRATE_SOURCES,
	MIGRATE_TARGET,
	WAITING_FOR_DEVICE,
	canStartHere,
	coverageOf,
	coverageRows,
	deviceUpdateNotice,
	emptyListingsLine,
	headStage,
	isDeviceImport,
	listRowLine,
	mayMigrate,
	migrateBody,
	migrateSource,
	presentStage,
	resourceRows,
	sourcesOn,
	stageOf,
	tally,
	targetAuthorship,
	termCoverageRows,
	termCoverageStrip
} from './sync-request';
import type {
	AuthorshipView,
	ConnectionView,
	SyncCoverageView,
	SyncRequestHead,
	SyncRequestView,
	SyncResourceView,
	SyncTermCoverage
} from './api';
import type { Marketplace } from './generated/vocab';

function resource(partial: Partial<SyncResourceView> = {}): SyncResourceView {
	return {
		ordinal: 0,
		locator: 'tes-1',
		state: 'canonicalised',
		failure_detail: null,
		...partial
	};
}

function request(partial: Partial<SyncRequestView> = {}): SyncRequestView {
	return {
		request: 'r-1',
		source: 'Tes',
		target: 'Tpt',
		disposition: 'migrate',
		intent: 'draft',
		state: 'pending',
		failure_detail: null,
		create_job: null,
		remove_job: null,
		resources: [],
		...partial
	};
}

const TERM_ZEROS: SyncTermCoverage = {
	terms_seen: 0,
	terms_mapped: 0,
	terms_unmapped: 0,
	terms_uncovered: 0
};

const ZEROS: SyncCoverageView = { rows: 0, ...TERM_ZEROS };

describe('the three states that must never be confused', () => {
	it('pending with nothing named is waiting for the device, not a result', () => {
		const stage = stageOf(request({ state: 'pending' }));
		expect(stage.kind).toBe('waiting_for_device');
		expect(presentStage(stage).headline).toBe(WAITING_FOR_DEVICE);
		expect(presentStage(stage).tone).not.toBe('bad');
	});

	it('enqueued with nothing named is an empty shop, not a failure', () => {
		const stage = stageOf(request({ state: 'enqueued' }));
		expect(stage.kind).toBe('nothing_to_import');
		const shown = presentStage(stage);
		expect(shown.headline).toBe('Your shop had nothing to import.');
		expect(shown.tone).toBe('ok');
	});

	it('failed is a failure and says what the server recorded', () => {
		const stage = stageOf(
			request({ state: 'failed', failure_detail: 'the shop refused the read' })
		);
		expect(stage.kind).toBe('failed');
		const shown = presentStage(stage);
		expect(shown.tone).toBe('bad');
		expect(shown.detail).toBe('the shop refused the read');
	});

	it('the three read differently from one another in every rendered field', () => {
		const shown = [
			presentStage(stageOf(request({ state: 'pending' }))),
			presentStage(stageOf(request({ state: 'enqueued' }))),
			presentStage(stageOf(request({ state: 'failed' })))
		];
		expect(new Set(shown.map((one) => one.label)).size).toBe(3);
		expect(new Set(shown.map((one) => one.headline)).size).toBe(3);
		expect(new Set(shown.map((one) => one.tone)).size).toBe(3);
	});

	it('a failure with nothing recorded says so rather than showing nothing', () => {
		const shown = presentStage(stageOf(request({ state: 'failed', failure_detail: null })));
		expect(shown.detail).not.toBe('');
	});
});

describe('the states that carry resources', () => {
	it('pending with resources named is queued rather than waiting for a device', () => {
		const stage = stageOf(
			request({ state: 'pending', resources: [resource({ state: 'pending' })] })
		);
		expect(stage.kind).toBe('queued');
		expect(presentStage(stage).headline).not.toBe(WAITING_FOR_DEVICE);
	});

	it('draining counts what has arrived so far', () => {
		const stage = stageOf(
			request({
				state: 'draining',
				resources: [resource({ ordinal: 0 }), resource({ ordinal: 1 })]
			})
		);
		expect(stage.kind).toBe('importing');
		expect(presentStage(stage).headline).toBe('Importing, 2 so far.');
	});

	it('enqueued with resources reports what was imported and what was skipped', () => {
		const stage = stageOf(
			request({
				state: 'enqueued',
				resources: [
					resource({ ordinal: 0 }),
					resource({ ordinal: 1, state: 'failed', failure_detail: 'no file on the listing' })
				]
			})
		);
		expect(stage.kind).toBe('imported');
		const shown = presentStage(stage);
		expect(shown.headline).toBe('1 listing imported.');
		expect(shown.detail).toContain('1 listing skipped');
	});

	it('the tally partitions the resources, so the three sum to the total', () => {
		const counted = tally([
			resource({ ordinal: 0 }),
			resource({ ordinal: 1, state: 'failed' }),
			resource({ ordinal: 2, state: 'pending' })
		]);
		expect(counted).toEqual({
			imported: 1,
			skipped: 1,
			unsettled: 1,
			unrecognised: 0,
			total: 3
		});
		expect(
			counted.imported + counted.skipped + counted.unsettled + counted.unrecognised
		).toBe(counted.total);
	});

	it("a skipped resource shows the device's own words, unaltered", () => {
		const rows = resourceRows(
			request({
				state: 'enqueued',
				resources: [
					resource({ state: 'failed', failure_detail: 'Tes answered 403 for this resource' })
				]
			})
		);
		expect(rows[0].reason).toBe('Tes answered 403 for this resource');
		expect(rows[0].label).toBe('Skipped');
		expect(rows[0].tone).toBe('bad');
	});
});

describe('coverage, where absent and zero are different facts', () => {
	it('an absent field is nothing to render', () => {
		expect(coverageOf(request())).toBeNull();
	});

	it('a null field is the same absence', () => {
		expect(coverageOf(request({ coverage: null }))).toBeNull();
	});

	it('zeros are a measurement and are rendered as zeros', () => {
		const coverage = coverageOf(request({ coverage: ZEROS }));
		expect(coverage).toEqual(ZEROS);
		expect(coverageRows(coverage as SyncCoverageView).map((row) => row.value)).toEqual([
			0, 0, 0, 0, 0
		]);
	});

	it("a resource's coverage counts terms only, not the row it is already on", () => {
		expect(termCoverageRows(TERM_ZEROS)).toHaveLength(4);
		expect(termCoverageRows(TERM_ZEROS).map((row) => row.label)).not.toContain(
			'Listings measured'
		);
	});

	it('every counter is written out, with its own label', () => {
		const rows = coverageRows({
			rows: 12,
			terms_seen: 40,
			terms_mapped: 33,
			terms_unmapped: 7,
			terms_uncovered: 5
		});
		expect(rows.map((row) => row.value)).toEqual([12, 40, 33, 7, 5]);
		expect(new Set(rows.map((row) => row.label)).size).toBe(5);
	});

	it('a resource carries its own coverage, absent and present alike', () => {
		const rows = resourceRows(
			request({
				state: 'enqueued',
				resources: [
					resource({ ordinal: 0 }),
					resource({ ordinal: 1, coverage: TERM_ZEROS })
				]
			})
		);
		expect(rows[0].coverage).toBeNull();
		expect(rows[1].coverage).toEqual(TERM_ZEROS);
	});

	it('a skipped resource is measured by nobody, so it renders no figures', () => {
		const rows = resourceRows(
			request({
				state: 'enqueued',
				resources: [resource({ state: 'failed', failure_detail: 'the device gave up' })]
			})
		);
		expect(rows[0].coverage).toBeNull();
		expect(rows[0].reason).toBe('the device gave up');
	});
});

describe('the device-version banner', () => {
	it('is absent while every machine can run the work', () => {
		expect(deviceUpdateNotice(request())).toBeNull();
		expect(deviceUpdateNotice(request({ waiting_for_device_version: null }))).toBeNull();
	});

	it('names the version the seller must reach', () => {
		const notice = deviceUpdateNotice(request({ waiting_for_device_version: '0.2.0' }));
		expect(notice).not.toBeNull();
		expect(notice).toContain('0.2.0');
		expect(notice).toContain('or later');
	});

	it('says nothing rather than naming an empty version', () => {
		expect(deviceUpdateNotice(request({ waiting_for_device_version: '' }))).toBeNull();
		expect(deviceUpdateNotice(request({ waiting_for_device_version: '  ' }))).toBeNull();
	});
});

function connection(marketplace: Marketplace, authorship?: AuthorshipView): ConnectionView {
	return {
		id: `c-${marketplace}`,
		marketplace,
		state: 'linked',
		status: 'connected',
		created_at: 0,
		updated_at: 0,
		...(authorship === undefined ? {} : { authorship })
	};
}

const DECLARED: AuthorshipView = {
	state: 'declared',
	name: 'A Teacher',
	attested_at: 0
};

describe('starting a migrate', () => {
	it('offers one only where the seller has a marketplace it can read from', () => {
		expect(migrateSource([connection('Tes')])).toBe('Tes');
		expect(migrateSource([])).toBeNull();
	});

	it('does not offer one from a marketplace a migrate cannot read', () => {
		expect(migrateSource([connection('Tpt')])).toBeNull();
		expect(migrateSource([connection('Etsy')])).toBeNull();
	});

	// Amended rather than deleted: the reason it states survives for the state
	// it was written about. A connection needing a fresh sign-in is still the
	// seller's shop and their own device is what discovers the sign-in is
	// needed, so a migrate is still offered from it. What the reason never
	// covered is a connection the seller gave up.
	it('offers one from a connection that needs a fresh sign-in, because the device decides', () => {
		const stale = {
			...connection('Tes'),
			state: 'needs_reauth',
			status: 'disconnected' as const
		};
		expect(migrateSource([stale])).toBe('Tes');
	});

	it('offers none from a marketplace the seller disconnected, or one revoked', () => {
		for (const state of ['unlinked', 'revoked']) {
			const off = { ...connection('Tes'), state, status: 'disconnected' as const };
			expect(migrateSource([off]), state).toBeNull();
		}
	});

	it('names the inventory of the marketplace the seller may have sold on', () => {
		expect(sourcesOn('Tes')).toEqual(['Tes']);
		expect(sourcesOn('Tpt')).toEqual([]);
	});

	it('submits a draft migrate naming no resources', () => {
		expect(migrateBody('Tes')).toEqual({
			source: 'Tes',
			target: 'Tpt',
			disposition: 'migrate',
			intent: 'draft',
			resources: []
		});
	});

	it('never submits a source that is also the target', () => {
		for (const source of MIGRATE_SOURCES) {
			expect(migrateBody(source).source).not.toBe(MIGRATE_TARGET);
		}
	});
});

describe('the authorship gate', () => {
	it('lets a migrate through only once a declaration is on record', () => {
		const standing = targetAuthorship([connection('Tes'), connection('Tpt', DECLARED)]);
		expect(standing).toEqual({ kind: 'declared', name: 'A Teacher' });
		expect(mayMigrate(standing)).toBe(true);
	});

	it('refuses while the seller has not declared', () => {
		const standing = targetAuthorship([
			connection('Tes'),
			connection('Tpt', { state: 'undeclared' })
		]);
		expect(standing).toEqual({ kind: 'undeclared' });
		expect(mayMigrate(standing)).toBe(false);
	});

	it('refuses where nothing is on record, without claiming the seller failed to declare', () => {
		expect(targetAuthorship([connection('Tes')])).toEqual({ kind: 'unrecorded' });
		expect(targetAuthorship([connection('Tpt')])).toEqual({ kind: 'unrecorded' });
		expect(mayMigrate({ kind: 'unrecorded' })).toBe(false);
	});

	it("reads the target's declaration rather than the source's", () => {
		const standing = targetAuthorship([connection('Tes', DECLARED), connection('Tpt')]);
		expect(mayMigrate(standing)).toBe(false);
	});

	it('says nothing is on record rather than accusing the seller', () => {
		expect(AUTHORSHIP_FIRST).toContain('on record');
		// Every way of putting the blame on the seller that the sentence could
		// drift into, not just the one phrasing it happens not to use. The
		// earlier assertion passed for "you failed to declare" and "you never
		// declared", which are the sentences it exists to reject.
		expect(AUTHORSHIP_FIRST).not.toMatch(
			/you (have not|haven't|never|failed|forgot|did not|didn't|neglected)/i
		);
		expect(AUTHORSHIP_FIRST).not.toMatch(/your (fault|mistake|error)/i);
		// And it must still tell them what to do about it.
		expect(AUTHORSHIP_FIRST).toMatch(/declare/i);
	});
});

describe('which requests a computer can still be asked to run', () => {
	it('a pending request naming nothing is the one a machine can pick up', () => {
		expect(canStartHere(request({ state: 'pending' }))).toBe(true);
	});

	it('a request already being run by some machine is not offered again', () => {
		expect(canStartHere(request({ state: 'draining', resources: [resource()] }))).toBe(false);
	});

	it('a finished request is not offered, however it finished', () => {
		expect(canStartHere(request({ state: 'enqueued' }))).toBe(false);
		expect(canStartHere(request({ state: 'enqueued', resources: [resource()] }))).toBe(false);
		expect(canStartHere(request({ state: 'failed' }))).toBe(false);
	});

	it('a sync that named its own resources is not a device import', () => {
		expect(
			canStartHere(request({ state: 'pending', resources: [resource({ state: 'pending' })] }))
		).toBe(false);
	});

	it('the waiting sentence tells the seller what to do, not only that they wait', () => {
		expect(WAITING_FOR_DEVICE).toContain('Teachouse app');
		expect(WAITING_FOR_DEVICE).toContain('start it from this page');
	});
});

describe('a state this console does not know', () => {
	it('degrades to saying so rather than blanking the page', () => {
		const view = { ...request(), state: 'cancelled' as unknown as SyncRequestView['state'] };
		const stage = stageOf(view);
		expect(stage.kind).toBe('unrecognised');
		const shown = presentStage(stage);
		expect(shown.headline).not.toBe('');
		expect(shown.detail).toContain('cancelled');
		expect(shown.tone).toBe('mut');
	});

	it('counts an unknown resource state rather than dropping it off the arithmetic', () => {
		const odd = {
			...resource(),
			state: 'quarantined' as unknown as SyncResourceView['state']
		};
		const counted = tally([resource(), odd]);
		expect(counted.unrecognised).toBe(1);
		expect(
			counted.imported + counted.skipped + counted.unsettled + counted.unrecognised
		).toBe(counted.total);
		expect(Number.isNaN(counted.total)).toBe(false);
	});

	it('labels an unknown resource state instead of rendering the word undefined', () => {
		const odd = {
			...resource(),
			state: 'quarantined' as unknown as SyncResourceView['state']
		};
		const rows = resourceRows({ ...request(), state: 'enqueued', resources: [odd] });
		expect(rows[0].label).not.toBe('undefined');
		expect(rows[0].label).not.toBe('');
		expect(rows[0].label.toLowerCase()).toContain('unrecognised');
	});

	it('an unknown state never claims the import succeeded or failed', () => {
		const view = { ...request(), state: 'cancelled' as unknown as SyncRequestView['state'] };
		const shown = presentStage(stageOf(view));
		expect(shown.tone).not.toBe('ok');
		expect(shown.tone).not.toBe('bad');
	});
});

describe('the branches a settled import can end in', () => {
	it('draining with nothing arrived yet still counts, rather than reading as empty', () => {
		const shown = presentStage(stageOf(request({ state: 'draining' })));
		expect(shown.headline).toBe('Importing, 0 so far.');
		expect(shown.label).toBe('Importing');
	});

	it('an import where every listing was skipped says so and is not toned as clean', () => {
		const stage = stageOf(
			request({
				state: 'enqueued',
				resources: [
					resource({ ordinal: 0, state: 'failed', failure_detail: 'no file' }),
					resource({ ordinal: 1, state: 'failed', failure_detail: 'no file' })
				]
			})
		);
		const shown = presentStage(stage);
		// The headline carries it, not the detail: a list row reads the headline
		// alone, so a count that only appears underneath never reaches the list.
		expect(shown.label).toBe('Nothing imported');
		expect(shown.headline).toBe('Nothing was imported: 2 listings skipped.');
		expect(shown.headline).not.toContain('0 listings imported');
		expect(shown.detail).toContain('Open this import');
		expect(shown.tone).toBe('run');
	});

	it('a resource left unsettled on a finished import is reported and changes the tone', () => {
		const stage = stageOf(
			request({
				state: 'enqueued',
				resources: [resource({ ordinal: 0 }), resource({ ordinal: 1, state: 'pending' })]
			})
		);
		const shown = presentStage(stage);
		expect(shown.detail).toContain('1 listing still unsettled.');
		expect(shown.tone).toBe('run');
	});

	it('only an import with nothing left over is toned as clean', () => {
		const clean = presentStage(
			stageOf(request({ state: 'enqueued', resources: [resource()] }))
		);
		expect(clean.tone).toBe('ok');
		expect(clean.detail).toBe('');
	});
});

describe('what the listings list says when it holds nothing', () => {
	it('follows the stage rather than always claiming something may still arrive', () => {
		const lines = new Map(
			(
				[
					['pending', 'waiting'],
					['draining', 'importing'],
					['enqueued', 'empty'],
					['failed', 'failed']
				] as const
			).map(([state, name]) => [
				name,
				emptyListingsLine(stageOf(request({ state: state as SyncRequestView['state'] })))
			])
		);
		expect(lines.get('waiting')).toBe('Nothing has arrived yet.');
		expect(lines.get('importing')).toBe('Nothing has arrived yet.');
		expect(lines.get('empty')).not.toBe('Nothing has arrived yet.');
		expect(lines.get('failed')).not.toBe('Nothing has arrived yet.');
	});

	it('never contradicts a settled stage by implying work is still coming', () => {
		for (const state of ['enqueued', 'failed'] as const) {
			const line = emptyListingsLine(stageOf(request({ state })));
			expect(line).not.toMatch(/yet|still|arriv/i);
		}
	});
});

describe('the coverage strip', () => {
	it('joins its figures without a trailing separator', () => {
		const strip = termCoverageStrip({
			terms_seen: 34,
			terms_mapped: 30,
			terms_unmapped: 4,
			terms_uncovered: 2
		});
		expect(strip).toBe('34 · 30 · 4 · 2');
		expect(strip.trimEnd().endsWith('·')).toBe(false);
	});

	it('names its four labels once, in the strip order', () => {
		expect(TERM_COVERAGE_LEGEND.split(' · ')).toHaveLength(4);
		expect(TERM_COVERAGE_LEGEND).toBe(
			termCoverageRows({
				terms_seen: 0,
				terms_mapped: 0,
				terms_unmapped: 0,
				terms_uncovered: 0
			})
				.map((row) => row.label)
				.join(' · ')
		);
	});
});

describe('the screen serves an import and says so for anything else', () => {
	it('knows a device import from a server-side sync', () => {
		expect(isDeviceImport(request({ disposition: 'migrate' }))).toBe(true);
		expect(isDeviceImport(request({ disposition: 'sync' }))).toBe(false);
	});

	it('the refusal names what the page is for rather than showing device copy', () => {
		expect(NOT_AN_IMPORT).toMatch(/this page follows/i);
		expect(NOT_AN_IMPORT).not.toMatch(/your device/i);
	});

	it('states where the seller files stay, which is the sentence the screen exists for', () => {
		expect(FILES_STAY_ON_YOUR_COMPUTER).toMatch(/your own computer/i);
		expect(FILES_STAY_ON_YOUR_COMPUTER).toMatch(/never reach our servers/i);
		// Narrower than "we keep nothing", because the thumbnail is kept.
		expect(FILES_STAY_ON_YOUR_COMPUTER).toMatch(/thumbnail/i);
	});
});

describe('the version banner and the start action, together', () => {
	const owed = { waiting_for_device_version: '0.2.0' };

	it('the banner speaks only where its sentence is true, which is the waiting stage', () => {
		expect(deviceUpdateNotice(request({ state: 'pending', ...owed }))).not.toBeNull();
		for (const state of ['draining', 'enqueued', 'failed'] as const) {
			expect(
				deviceUpdateNotice(request({ state, resources: [resource()], ...owed }))
			).toBeNull();
		}
	});

	it('a finished import is never told that no machine can run it', () => {
		const done = request({ state: 'enqueued', resources: [resource()], ...owed });
		expect(deviceUpdateNotice(done)).toBeNull();
		expect(presentStage(stageOf(done)).headline).toBe('1 listing imported.');
	});

	it('the start action is withheld while a version is owed', () => {
		expect(canStartHere(request({ state: 'pending' }))).toBe(true);
		expect(canStartHere(request({ state: 'pending', ...owed }))).toBe(false);
	});

	it('the banner and the button are never both offered', () => {
		const waiting = request({ state: 'pending', ...owed });
		expect(deviceUpdateNotice(waiting)).not.toBeNull();
		expect(canStartHere(waiting)).toBe(false);
	});
});

function head(partial: Partial<SyncRequestHead> = {}): SyncRequestHead {
	return {
		request: 'r-1',
		source: 'Tes',
		target: 'Tpt',
		disposition: 'migrate',
		intent: 'draft',
		state: 'pending',
		created_at: 0,
		resources_total: 0,
		resources_failed: 0,
		...partial
	};
}

describe('the list of a seller own imports', () => {
	it('stages a row from its counts, so the list and the page agree', () => {
		expect(headStage(head()).kind).toBe('waiting_for_device');
		expect(headStage(head({ state: 'enqueued' })).kind).toBe('nothing_to_import');
		expect(headStage(head({ state: 'failed' })).kind).toBe('failed');
		expect(
			headStage(head({ state: 'draining', resources_total: 3 })).kind
		).toBe('importing');
	});

	it('reads imported as what did not fail', () => {
		const shown = presentStage(
			headStage(head({ state: 'enqueued', resources_total: 5, resources_failed: 2 }))
		);
		expect(shown.headline).toBe('3 listings imported.');
		expect(shown.detail).toContain('2 listings skipped');
	});

	it('never reports a negative count if the server disagrees with itself', () => {
		const shown = presentStage(
			headStage(head({ state: 'enqueued', resources_total: 1, resources_failed: 4 }))
		);
		// Clamped to zero rather than negative, and read as nothing imported.
		expect(shown.headline).not.toContain('-');
		expect(shown.headline).toBe('Nothing was imported: 4 listings skipped.');
	});

	it('still reads a partly skipped import by what arrived', () => {
		const shown = presentStage(
			headStage(head({ state: 'enqueued', resources_total: 5, resources_failed: 2 }))
		);
		expect(shown.label).toBe('Imported');
		expect(shown.headline).toBe('3 listings imported.');
		expect(shown.tone).toBe('run');
	});

	it('says nothing was imported without inventing a reason it does not hold', () => {
		const shown = presentStage(
			headStage(head({ state: 'enqueued', resources_total: 2, resources_failed: 2 }))
		);
		// The head counts; it does not carry any resource's reason, so the row
		// points at the page that does rather than implying it has read them.
		expect(shown.headline).toBe('Nothing was imported: 2 listings skipped.');
		expect(shown.detail).toBe('Open this import to see what was recorded against each one.');
	});

	it('degrades on a state it does not know, exactly as the page does', () => {
		expect(
			headStage(head({ state: 'cancelled' as unknown as SyncRequestHead['state'] })).kind
		).toBe('unrecognised');
	});
});

describe('an authorship declaration that arrives as null', () => {
	it('reads as absent rather than throwing, as every other optional here does', () => {
		const nulled = {
			...connection('Tpt'),
			authorship: null as unknown as ConnectionView['authorship']
		};
		expect(() => targetAuthorship([nulled])).not.toThrow();
		expect(targetAuthorship([nulled])).toEqual({ kind: 'unrecorded' });
	});
});

describe('the sentence a list row shows', () => {
	it('never tells a seller to start the import from the page they are reading', () => {
		const line = listRowLine(stageOf(request({ state: 'pending' })));
		expect(line).toBe(WAITING_IN_LIST);
		expect(line).not.toBe(WAITING_FOR_DEVICE);
		expect(line).not.toMatch(/from this page/i);
	});

	it('points at the request, which is where starting actually lives', () => {
		expect(WAITING_IN_LIST).toMatch(/open this import/i);
	});

	it('is the stage headline for every stage that is not the wait', () => {
		for (const state of ['draining', 'enqueued', 'failed'] as const) {
			const stage = stageOf(request({ state, resources: [resource()] }));
			expect(listRowLine(stage)).toBe(presentStage(stage).headline);
		}
	});

	it('the request page keeps the sentence that belongs to it', () => {
		expect(presentStage(stageOf(request({ state: 'pending' }))).headline).toBe(
			WAITING_FOR_DEVICE
		);
		expect(WAITING_FOR_DEVICE).toMatch(/from this page/i);
	});
});

describe('what counts as the import this screen describes', () => {
	it('needs both the device-branch source and the migrate', () => {
		expect(isDeviceImport(request({ source: 'Tes', disposition: 'migrate' }))).toBe(true);
		expect(isDeviceImport(request({ source: 'Tes', disposition: 'sync' }))).toBe(false);
	});

	it('refuses a migrate whose source we work ourselves, which the disposition alone would admit', () => {
		expect(isDeviceImport(request({ source: 'Etsy', disposition: 'migrate' }))).toBe(false);
	});

	it('the refusal copy is true whichever way the request failed the test', () => {
		expect(NOT_AN_IMPORT).not.toMatch(/\bis a sync\b/i);
		expect(NOT_AN_IMPORT).not.toMatch(/your device/i);
		expect(NOT_AN_IMPORT).toMatch(/your own computer reads/i);
	});
});
