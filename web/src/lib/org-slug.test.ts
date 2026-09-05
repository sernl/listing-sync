import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
	RESERVED_SLUGS,
	SLUG_MAX_CHARS,
	SLUG_MIN_CHARS,
	checkOrgSlug,
	nameFromSlug
} from './org-slug';

describe('the organisation-slug check', () => {
	it('accepts each well-formed shape and stores it as typed', () => {
		for (const raw of [
			'abc',
			'riverbend',
			'riverbend-resources',
			'a-b-c',
			'year-6-maths',
			'123',
			'z'.repeat(SLUG_MAX_CHARS)
		]) {
			expect(checkOrgSlug(raw), `accepted: ${raw}`).toEqual({ accepted: true, slug: raw });
		}
	});

	it('normalises case and whitespace rather than refusing them', () => {
		expect(checkOrgSlug('  RiverBend-Resources  ')).toEqual({
			accepted: true,
			slug: 'riverbend-resources'
		});
	});

	it('refuses each violation class on its own, matching the server case by case', () => {
		const cases: [string, string][] = [
			['', 'empty'],
			['   ', 'empty'],
			['ab', 'too-short'],
			['a'.repeat(SLUG_MAX_CHARS + 1), 'too-long'],
			['-abc', 'hyphens'],
			['abc-', 'hyphens'],
			['ab--c', 'hyphens'],
			['ab_c', 'characters'],
			['ab c', 'characters'],
			['ab.c', 'characters'],
			['ab/c', 'characters'],
			['café', 'characters'],
			['рив', 'characters'],
			['0123456789abcdef0123456789abcdef', 'reserved']
		];
		for (const [raw, problem] of cases) {
			const verdict = checkOrgSlug(raw);
			expect(verdict.accepted, `refused: ${JSON.stringify(raw)}`).toBe(false);
			expect(verdict.accepted === false && verdict.problem, JSON.stringify(raw)).toBe(problem);
		}
	});

	it('states the bounds exactly rather than off by one', () => {
		// `z` rather than `a`: a run of `a` is hexadecimal, so a 32-character
		// one is refused by the unhyphenated-UUID rule and would prove the
		// ceiling wrong for a reason that has nothing to do with length.
		expect(checkOrgSlug('z'.repeat(SLUG_MIN_CHARS)).accepted).toBe(true);
		expect(checkOrgSlug('z'.repeat(SLUG_MIN_CHARS - 1)).accepted).toBe(false);
		expect(checkOrgSlug('z'.repeat(SLUG_MAX_CHARS)).accepted).toBe(true);
		expect(checkOrgSlug('z'.repeat(SLUG_MAX_CHARS + 1)).accepted).toBe(false);
	});

	it('refuses a hexadecimal string only at the full identifier length', () => {
		expect(checkOrgSlug('0123456789abcdef0123456789abcde').accepted).toBe(true);
		expect(checkOrgSlug('0123456789abcdef0123456789abcdef').accepted).toBe(false);
	});

	it('refuses every reserved word for being reserved', () => {
		for (const reserved of RESERVED_SLUGS) {
			const verdict = checkOrgSlug(reserved);
			expect(verdict.accepted, reserved).toBe(false);
			expect(verdict.accepted === false && verdict.problem, reserved).toBe('reserved');
		}
	});

	it('keeps the reserved list sorted, lowercase and reachable', () => {
		expect([...RESERVED_SLUGS].sort()).toEqual([...RESERVED_SLUGS]);
		for (const reserved of RESERVED_SLUGS) {
			expect(reserved).toBe(reserved.toLowerCase());
			// A word shorter than the floor is already unreachable, so reserving
			// it would enforce nothing. The Rust list carries the same assertion.
			expect(reserved.length, reserved).toBeGreaterThanOrEqual(SLUG_MIN_CHARS);
		}
	});
});

describe('the display name a slug proposes', () => {
	it('capitalises each hyphen-separated word', () => {
		expect(nameFromSlug('riverbend-resources')).toBe('Riverbend Resources');
		expect(nameFromSlug('year-6-maths')).toBe('Year 6 Maths');
		expect(nameFromSlug('riverbend')).toBe('Riverbend');
	});

	it('proposes nothing from nothing', () => {
		expect(nameFromSlug('')).toBe('');
	});
});

// The list is policy, and policy that is maintained by eye drifts silently:
// a route added on Monday is a slug a seller can claim until someone happens
// to notice. These two tests are what make the list's own comment true --
// the first derives the product's top-level paths from the route table so a
// new route fails here, the second holds the client's copy against the Rust
// authority so the two cannot part company.

function dirAt(relative: string): string[] {
	return readdirSync(fileURLToPath(new URL(relative, import.meta.url)));
}

/** Every top-level path this product serves, read from the trees that define
 *  them rather than restated. A name that could not be a slug anyway -- a
 *  filename with a dot, anything under three characters such as the API's own
 *  `v1` -- is dropped, because reserving an unclaimable handle enforces
 *  nothing and would fail the reachability assertion above. */
function servedTopLevelPaths(): string[] {
	const routes = dirAt('../routes').filter((name) => !name.startsWith('+'));
	const statics = dirAt('../../static');
	const landing = dirAt('../../../apps/landing/src/pages')
		.map((file) => file.replace(/\.[^.]+$/, ''))
		.filter((stem) => stem !== 'index');
	// Shape only, deliberately not `accepted`. `checkOrgSlug` applies the
	// reserved list as well as the shape rule, so filtering on its verdict
	// would drop every name this test exists to check the moment that name was
	// reserved -- the assertion would go green by having nothing left to look
	// at rather than by finding nothing wrong. Asking instead whether the only
	// objection is that it is reserved keeps the real validator in the loop
	// without letting it hide the subject.
	return [...new Set([...routes, ...statics, ...landing])].filter((name) => {
		const verdict = checkOrgSlug(name);
		return verdict.accepted || verdict.problem === 'reserved';
	});
}

describe('the reserved list against the paths this product actually serves', () => {
	it('reserves every top-level path the route table defines', () => {
		const unreserved = servedTopLevelPaths().filter((name) => !RESERVED_SLUGS.includes(name));
		expect(
			unreserved,
			'a top-level path a seller could claim as their own name: add it to RESERVED_SLUGS ' +
				'in crates/tam-api/src/org.rs and to the mirror in org-slug.ts'
		).toEqual([]);
	});

	it('derives from a route table that is actually there', () => {
		// Without this, a moved directory makes the test above pass by reading
		// nothing -- the failure mode where a gate goes green because it
		// stopped looking rather than because it found nothing wrong.
		const served = servedTopLevelPaths();
		expect(served.length).toBeGreaterThan(20);
		expect(served).toContain('settings');
		expect(served).toContain('pricing');
	});

	it('carries the same words as the Rust authority, in the same order', () => {
		const rust = readFileSync(
			fileURLToPath(new URL('../../../crates/tam-api/src/org.rs', import.meta.url)),
			'utf8'
		);
		const block = rust.split('pub const RESERVED_SLUGS')[1]?.split('];')[0] ?? '';
		const declared = [...block.matchAll(/"([a-z0-9-]+)"/g)].map((match) => match[1]);
		expect(declared.length, 'the Rust list was not found to compare against').toBeGreaterThan(0);
		expect(RESERVED_SLUGS).toEqual(declared);
	});
});
