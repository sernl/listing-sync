import { describe, expect, it } from 'vitest';
import {
	BODY_MAX_BYTES,
	bodyRefusal,
	dirty,
	draftRefusal,
	imageMarkdown,
	insertAt,
	slugRefusal,
	slugify,
	titleRefusal
} from './editor';

describe('a guide slug', () => {
	it('takes lowercase kebab', () => {
		expect(slugRefusal('connecting-a-marketplace')).toBeNull();
		expect(slugRefusal('tpt-2026')).toBeNull();
	});

	it('refuses the shapes the table would refuse', () => {
		// Each of these is a check-constraint violation rather than a matter of
		// taste, which is why the form refuses it instead of sending it.
		expect(slugRefusal('')).not.toBeNull();
		expect(slugRefusal('Connecting')).not.toBeNull();
		expect(slugRefusal('two words')).not.toBeNull();
		expect(slugRefusal('trailing-')).not.toBeNull();
		expect(slugRefusal('-leading')).not.toBeNull();
		expect(slugRefusal('double--hyphen')).not.toBeNull();
		expect(slugRefusal('a'.repeat(81))).not.toBeNull();
	});

	it('suggests one from a title without ever producing a slug it would refuse', () => {
		for (const title of [
			'Connecting a marketplace',
			'  Import your TpT portfolio!  ',
			'Pricing & VAT (2026)',
			'Émile’s guide',
			'a'.repeat(200)
		]) {
			expect(slugRefusal(slugify(title)), title).toBeNull();
		}
	});

	it('suggests nothing from a title that has no slug in it', () => {
		// Not a refusal: the operator types one. The suggestion just has to not
		// be a broken slug.
		expect(slugify('!!!')).toBe('');
	});
});

describe('a guide title', () => {
	it('has to say something', () => {
		expect(titleRefusal('   ')).not.toBeNull();
		expect(titleRefusal('Connecting a marketplace')).toBeNull();
	});

	it('stops at the column width', () => {
		expect(titleRefusal('a'.repeat(120))).toBeNull();
		expect(titleRefusal('a'.repeat(121))).not.toBeNull();
	});
});

describe('a guide body', () => {
	it('may be empty, because that is how a guide starts', () => {
		expect(bodyRefusal('')).toBeNull();
	});

	it('is bounded in bytes rather than characters', () => {
		// Four bytes each, so a body a quarter of the limit in characters is
		// exactly at it. Counting characters would have accepted four times the
		// column's limit and met a 422 from the server.
		const emoji = '🙂'.repeat(BODY_MAX_BYTES / 4);
		expect(bodyRefusal(emoji)).toBeNull();
		expect(bodyRefusal(`${emoji}🙂`)).not.toBeNull();
	});

	it('is what the draft refusal reports once the title is fine', () => {
		expect(draftRefusal({ title: 'Fine', body: '🙂'.repeat(BODY_MAX_BYTES), status: 'draft' }))
			.not.toBeNull();
		expect(draftRefusal({ title: 'Fine', body: 'Short', status: 'published' })).toBeNull();
	});
});

describe('an inserted image', () => {
	it('points at the reader route, which every seller can read', () => {
		// The operator route would 404 for everybody but the operator, and the
		// image would be missing from the published guide rather than from the
		// editor, which is where nobody would notice it.
		expect(imageMarkdown('a'.repeat(64))).toBe(`![](/v1/guides/images/${'a'.repeat(64)})`);
	});

	it('lands at the caret and leaves it after what was inserted', () => {
		const { text, caret } = insertAt('before after', 7, 7, 'X');
		expect(text).toBe('before Xafter');
		expect(caret).toBe(8);
	});

	it('replaces a selection rather than wrapping it', () => {
		expect(insertAt('keep this away', 5, 9, 'that').text).toBe('keep that away');
	});

	it('clamps a caret the textarea no longer has', () => {
		// The selection is read off the element, and the value can have been
		// shortened by a save in between; an unclamped slice would silently
		// append instead of throwing.
		expect(insertAt('short', 99, 120, '!')).toEqual({ text: 'short!', caret: 6 });
		expect(insertAt('short', -3, 2, '!').text).toBe('!ort');
	});
});

describe('an unsaved guide', () => {
	const saved = { title: 'T', body: 'B', status: 'draft' } as const;

	it('is clean against its own saved copy', () => {
		expect(dirty(saved, { ...saved })).toBe(false);
	});

	it('notices each field on its own, status included', () => {
		expect(dirty(saved, { ...saved, title: 'T2' })).toBe(true);
		expect(dirty(saved, { ...saved, body: 'B2' })).toBe(true);
		expect(dirty(saved, { ...saved, status: 'published' })).toBe(true);
	});
});
