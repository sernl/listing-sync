import { describe, expect, it } from 'vitest';
import { sanitiseHtml } from '$lib/rich-text';
import {
	audienceSummary,
	countLine,
	countsLine,
	sanitiseMailHtml,
	usesVariable,
	validDraft
} from './mail';

const HANDLE = 'ab'.repeat(32);
const STORED = `/v1/mail/images/${HANDLE}`;

describe('the email allow-list', () => {
	it('keeps the two heading levels an email may carry', () => {
		expect(sanitiseMailHtml('<h2>News</h2><h3>Also</h3><p>Words</p>')).toBe(
			'<h2>News</h2><h3>Also</h3><p>Words</p>'
		);
	});

	it('reads any other heading as a paragraph', () => {
		expect(sanitiseMailHtml('<h1>Big</h1><h4>Small</h4>')).toBe('<p>Big</p><p>Small</p>');
	});

	it('keeps a picture from our own store and from an https address', () => {
		expect(sanitiseMailHtml(`<p><img src="${STORED}" alt="A chart"></p>`)).toBe(
			`<p><img src="${STORED}" alt="A chart"></p>`
		);
		expect(sanitiseMailHtml('<img src="https://example.test/a.png">')).toBe(
			'<p><img src="https://example.test/a.png" alt=""></p>'
		);
	});

	it('drops a picture from anywhere else, and every attribute but src and alt', () => {
		expect(sanitiseMailHtml('<p>Hi<img src="x" onerror="alert(1)"></p>')).toBe('<p>Hi</p>');
		expect(sanitiseMailHtml('<p><img src="http://example.test/a.png"></p>')).toBe('');
		expect(sanitiseMailHtml(`<img src="/v1/mail/images/${HANDLE}0">`)).toBe('');
		expect(sanitiseMailHtml('<img src="javascript:alert(1)">')).toBe('');
		expect(
			sanitiseMailHtml(`<img src="${STORED}" onerror="alert(1)" alt="x" style="width:9px">`)
		).toBe(`<p><img src="${STORED}" alt="x"></p>`);
	});

	it('drops scripts whole and unsafe links to their words', () => {
		expect(sanitiseMailHtml('<p>Hi<script>alert(1)</script></p>')).toBe('<p>Hi</p>');
		expect(sanitiseMailHtml('<p><a href="javascript:alert(1)">Open</a></p>')).toBe('<p>Open</p>');
		expect(sanitiseMailHtml('<p><a href="https://teachouse.test">Open</a></p>')).toBe(
			'<p><a href="https://teachouse.test">Open</a></p>'
		);
	});

	it('leaves the variables as the words they are', () => {
		expect(sanitiseMailHtml('<p>Hello @first_name, from @org.</p><p>@link</p>')).toBe(
			'<p>Hello @first_name, from @org.</p><p>@link</p>'
		);
	});

	it('gives its own answer back unchanged', () => {
		const once = sanitiseMailHtml(
			`<h2>Tom &amp; Jerry</h2><p><b>Hi</b> <img src="https://e.test/a.png?x=1&amp;y=2" alt="A &quot;b&quot;"></p><ul><li>One</li></ul>`
		);
		expect(sanitiseMailHtml(once)).toBe(once);
		expect(once).toContain('src="https://e.test/a.png?x=1&amp;y=2"');
	});

	it('does not widen the description’s list', () => {
		expect(sanitiseHtml(`<h2>News</h2><p><img src="${STORED}">Words</p>`)).toBe(
			'<p>News</p><p>Words</p>'
		);
	});
});

describe('a variable in the body', () => {
	it('is found as a word of its own', () => {
		expect(usesVariable('<p>Open @link now</p>', '@link')).toBe(true);
		expect(usesVariable('<p>@link</p>', '@link')).toBe(true);
	});

	it('is not found inside a longer token or an address', () => {
		expect(usesVariable('<p>@linked</p>', '@link')).toBe(false);
		expect(usesVariable('<p>me@link.test</p>', '@link')).toBe(false);
		expect(usesVariable('<p>@first_name</p>', '@name')).toBe(false);
	});
});

describe('a draft that can send', () => {
	const draft = {
		subject: 'New this week',
		body_html: '<p>Hello</p>',
		link_url: null,
		link_label: null
	};

	it('passes when it has a subject and a body', () => {
		expect(validDraft(draft)).toBeNull();
	});

	it('needs a subject of at most 200 characters', () => {
		expect(validDraft({ ...draft, subject: '   ' })).toBe('Write a subject.');
		expect(validDraft({ ...draft, subject: 'é'.repeat(200) })).toBeNull();
		expect(validDraft({ ...draft, subject: 'a'.repeat(201) })).toMatch(/200 characters/);
	});

	it('needs a body that shows something', () => {
		expect(validDraft({ ...draft, body_html: '<p> </p><p><br></p>' })).toBe('Write the email.');
		expect(validDraft({ ...draft, body_html: `<p><img src="${STORED}"></p>` })).toBeNull();
	});

	it('needs an https address when the body uses @link', () => {
		const linked = { ...draft, body_html: '<p>@link</p>' };
		expect(validDraft(linked)).toMatch(/@link/);
		expect(validDraft({ ...linked, link_url: 'http://teachouse.test' })).toMatch(/https/);
		expect(validDraft({ ...linked, link_url: 'https://teachouse.test' })).toBeNull();
	});

	it('refuses an address that is not https even when @link is unused', () => {
		expect(validDraft({ ...draft, link_url: 'javascript:alert(1)' })).toMatch(/https/);
	});
});

describe('the sentences', () => {
	it('counts the audience', () => {
		expect(countLine(0)).toBe('Nobody matches these filters.');
		expect(countLine(1)).toBe('1 seller will get this.');
		expect(countLine(128)).toBe('128 sellers will get this.');
		expect(countLine(1280)).toBe('1,280 sellers will get this.');
	});

	it('names the audience', () => {
		expect(
			audienceSummary({ segment: 'subscriber', exclude_operators: true, verified_only: false })
		).toBe('Sellers on Sync, admins left out, any address.');
	});

	it('leaves out the states nobody is in', () => {
		expect(countsLine({ total: 5, sent: 3, failed: 0, queued: 2, skipped: 0 })).toBe(
			'3 sent, 2 queued.'
		);
		expect(countsLine({ total: 0, sent: 0, failed: 0, queued: 0, skipped: 0 })).toBe('Nobody.');
	});
});
