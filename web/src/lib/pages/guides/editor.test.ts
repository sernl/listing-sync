import { describe, expect, it } from 'vitest';
import {
	BODY_MAX_BYTES,
	bodyRefusal,
	dirty,
	editRefusal,
	footnoteIdentifiers,
	imageMarkdown,
	insertAt,
	loadsRemoteImages,
	markGuide,
	nextFootnoteId,
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

	it('is what the edit refusal reports once the title is fine', () => {
		expect(
			editRefusal({
				title: 'Fine',
				body: '🙂'.repeat(BODY_MAX_BYTES),
				topic_id: null,
				tag_ids: []
			})
		).not.toBeNull();
		expect(
			editRefusal({ title: 'Fine', body: 'Short', topic_id: null, tag_ids: [] })
		).toBeNull();
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
	const saved = { title: 'T', body: 'B', topic_id: 'topic-1', tag_ids: ['a', 'b'] };

	it('is clean against its own saved copy', () => {
		expect(dirty(saved, { ...saved })).toBe(false);
	});

	it('notices each field on its own', () => {
		expect(dirty(saved, { ...saved, title: 'T2' })).toBe(true);
		expect(dirty(saved, { ...saved, body: 'B2' })).toBe(true);
		expect(dirty(saved, { ...saved, topic_id: null })).toBe(true);
		expect(dirty(saved, { ...saved, tag_ids: ['a'] })).toBe(true);
	});

	it('reads a reordered tag set as the same edit', () => {
		// The server stores a set. Reading an order as an edit would leave the
		// guide permanently dirty, and autosave sending the same tags forever.
		expect(dirty(saved, { ...saved, tag_ids: ['b', 'a'] })).toBe(false);
	});
});

describe('a footnote', () => {
	it('takes an identifier the body does not already hold', () => {
		expect(nextFootnoteId('')).toBe('fn1');
		expect(nextFootnoteId('a[^fn1] b\n\n[^fn1]: note')).toBe('fn2');
		// The collision that matters: a gap in the sequence is still a
		// collision on the identifier that exists, and reusing `fn2` would
		// merge two notes into whichever the renderer read first.
		expect(nextFootnoteId('[^fn2] and [^fn1]')).toBe('fn3');
		expect(footnoteIdentifiers('[^a] [^b]: x')).toEqual(new Set(['a', 'b']));
	});

	it('marks the selected phrase rather than replacing it, and opens the note', () => {
		const marked = markGuide('Rates vary.', 0, 5, 'footnote');
		expect(marked.text).toBe('Rates[^fn1] vary.\n\n[^fn1]: ');
		// The caret is in the definition, which is the part still to write.
		expect(marked.start).toBe(marked.text.length);
		expect(marked.end).toBe(marked.text.length);
	});

	it('does not stack blank lines before the definition', () => {
		expect(markGuide('Text\n\n', 4, 4, 'footnote').text).toBe('Text[^fn1]\n\n[^fn1]: ');
	});
});

describe('the guide toolbar', () => {
	it('changes a heading level rather than prefixing a second one', () => {
		const once = markGuide('Title', 0, 0, 'heading');
		expect(once.text).toBe('## Title');
		expect(markGuide(once.text, 0, 0, 'subheading').text).toBe('### Title');
	});

	it('marks the whole lines a selection touches and keeps them selected', () => {
		const marked = markGuide('one\ntwo\nthree', 5, 6, 'heading');
		expect(marked.text).toBe('one\n## two\nthree');
		expect(marked.text.slice(marked.start, marked.end)).toBe('## two');
	});

	it('keeps a selected phrase as a link label and selects the address', () => {
		const marked = markGuide('see the docs here', 8, 12, 'link');
		expect(marked.text).toBe('see the [docs](https://) here');
		expect(marked.text.slice(marked.start, marked.end)).toBe('https://');
	});

	it('selects the label where there was no selection to keep', () => {
		const marked = markGuide('', 0, 0, 'link');
		expect(marked.text).toBe('[link text](https://)');
		expect(marked.text.slice(marked.start, marked.end)).toBe('link text');
	});

	it('writes a picture by address with the address selected', () => {
		const marked = markGuide('a b', 2, 3, 'image');
		expect(marked.text).toBe('a ![b](https://)');
		expect(marked.text.slice(marked.start, marked.end)).toBe('https://');
	});
});

describe('the external-picture disclosure', () => {
	it('recognises the scheme however it is spelled', () => {
		// The renderer permits a scheme case-insensitively and writes the
		// address back as it was typed, so a lowercase-only test loads a
		// third-party picture and says nothing about it.
		for (const scheme of ['https:', 'HTTPS:', 'HttpS:']) {
			expect(
				loadsRemoteImages(
					`<p><img src="${scheme}//img.example/a.png" alt="" referrerpolicy="no-referrer" loading="lazy" /></p>`
				),
				scheme
			).toBe(true);
		}
	});

	it('says nothing about pictures this site serves itself', () => {
		expect(
			loadsRemoteImages('<img src="/v1/guides/images/abc" alt="" referrerpolicy="no-referrer" />')
		).toBe(false);
		expect(loadsRemoteImages('<p>No picture at all.</p>')).toBe(false);
	});
});
