// The browser half of the equivalence proof.
//
// `crates/tam-core-wasm/fixtures/verdicts.json` records what the native path
// decides for each fixture draft — the path `POST /v1/authoring/check` runs —
// and a Rust test asserts that recording is current. This replays the same
// drafts through the compiled module and asserts the same answers, so neither
// side can drift without one of the two going red.
//
// The module is initialised from bytes rather than from the asset URL: there
// is no fetch here to serve one, and `initSync` takes bytes for exactly this.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { CheckView, DraftInput } from '$lib/api';
import { CoreFailure } from '$lib/core';
import { loadCoreForTest } from '$lib/core/testing';

const root = new URL('../../../../', import.meta.url);
const read = (path: string) => readFileSync(new URL(path, root), 'utf8');

const drafts: Record<string, DraftInput> = JSON.parse(
	read('crates/tam-core-wasm/fixtures/drafts.json')
);
const recorded: Record<string, CheckView> = JSON.parse(
	read('crates/tam-core-wasm/fixtures/verdicts.json')
);

const core = loadCoreForTest();

describe('the compiled core answers what the server answers', () => {
	const names = Object.keys(drafts);

	it('has fixtures to compare, so a passing suite is not an empty one', () => {
		expect(names.length).toBeGreaterThan(10);
		expect(Object.keys(recorded).sort()).toEqual(names.sort());
	});

	it.each(names)('agrees with the server about %s', (name) => {
		expect(core.checkDraft(drafts[name])).toEqual(recorded[name]);
	});

	it('exercises both outcomes, so the comparison is not vacuous', () => {
		const verdicts = names.map((name) => recorded[name].submittable);
		expect(verdicts).toContain(true);
		expect(verdicts).toContain(false);
	});
});

describe('the caps the module holds', () => {
	it('are the committed capture, with the contradicted one left unmeasured', () => {
		// Four grades and six tags are what the capture states. Subject Area is
		// null because a create TPT accepted posted four against a form that
		// says three, so the number is unmeasured rather than unlimited.
		expect(core.selectionCaps()).toEqual({
			grades: 4,
			subject_areas: null,
			tags: 6,
			formats: 3,
			thumbnails: 4
		});
	});

	it('refuse nothing when a caller states none, which is what unmeasured means', () => {
		// The same draft, decided twice: against the capture's own caps, and
		// against a caps object that measures nothing. An absent cap is not a
		// cap of zero and not an unlimited one; it is a number nobody holds, so
		// there is nothing to refuse against.
		const overCap = drafts.grades_over_cap;
		expect(core.checkDraft(overCap).submittable).toBe(false);
		expect(core.checkDraft(overCap, {}).submittable).toBe(true);
	});
});

describe('the projection preview', () => {
	it('states a real truncation loss where the target declares a cap', () => {
		const long = { name: 'Fractions '.repeat(20), description: 'body', free: true };
		const title = core.projectPreview(long, 'Etsy').rows.find((row) => row.key === 'title');
		expect(title?.cap).toEqual({ limit: 140, unit: 'codepoints' });
		expect(title?.loss).toContain('dropped');
	});

	it('states no loss where nothing is measured, rather than implying none exists', () => {
		const rows = core.projectPreview({ name: 'Fractions', free: true }, 'Tes').rows;
		expect(rows.every((row) => row.cap === null && row.loss === null)).toBe(true);
	});

	it('names the axes it does not decide, because their relation lives in Postgres', () => {
		expect(core.projectPreview({ free: true }, 'Tes').undecided_axes).toContain('subject');
	});
});

describe('a boundary failure', () => {
	it('is never a verdict, because a verdict with no refusals reads as approval', () => {
		expect(() => core.checkDraft(JSON.parse('{"name":1}'))).toThrow(CoreFailure);
	});

	it('names an unknown marketplace rather than guessing one', () => {
		// @ts-expect-error the point of the test is a value the type forbids
		expect(() => core.projectPreview({}, 'Nowhere')).toThrow(/not a marketplace/);
	});
});
