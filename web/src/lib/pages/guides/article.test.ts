import { describe, expect, it } from 'vitest';
import type { GuideHeadView, GuideTaxon } from '$lib/api';
import { groupBySection, nextGuide, outline } from './article';

const topic = (slug: string): GuideTaxon => ({ id: `t-${slug}`, slug, name: slug, retired: false });

const guide = (slug: string, filed: GuideTaxon | null): GuideHeadView => ({
	id: slug,
	slug,
	title: slug,
	status: 'published',
	updated_at: 0,
	revision: 1,
	topic: filed,
	tags: []
});

describe('the guide index sections', () => {
	it('puts the five sections in reading order, an unknown topic after them and unfiled guides last', () => {
		const sections = groupBySection([
			guide('none', null),
			guide('plans', topic('billing')),
			guide('odd', topic('something-new')),
			guide('import', topic('getting-started')),
			guide('export', topic('catalogue'))
		]);
		expect(sections.map((section) => section.slug)).toEqual([
			'getting-started',
			'catalogue',
			'billing',
			'something-new',
			null
		]);
	});

	it('keeps the order the server answered in inside a section', () => {
		const [only] = groupBySection([guide('b', topic('catalogue')), guide('a', topic('catalogue'))]);
		expect(only.guides.map((row) => row.slug)).toEqual(['b', 'a']);
	});

	it('reads the next guide in section order, across a section boundary, and none after the last', () => {
		const list = [guide('plans', topic('billing')), guide('import', topic('getting-started'))];
		expect(nextGuide(list, 'import')?.slug).toBe('plans');
		expect(nextGuide(list, 'plans')).toBeNull();
		expect(nextGuide(list, 'missing')).toBeNull();
	});
});

describe('a guide outline', () => {
	it('anchors each second-level heading, decoding its words for the contents and keeping them unique', () => {
		const { html, headings } = outline(
			'<h2>Before <code>you</code> start</h2><p>x</p><h2>Q &amp; A</h2><h2>Q &amp; A</h2><h3>Deep</h3>'
		);
		expect(headings).toEqual([
			{ id: 'gd-before-you-start', text: 'Before you start' },
			{ id: 'gd-q-a', text: 'Q & A' },
			{ id: 'gd-q-a-2', text: 'Q & A' }
		]);
		expect(html).toContain('<h2 id="gd-before-you-start">Before <code>you</code> start</h2>');
		expect(html).toContain('<h3>Deep</h3>');
	});

	it('captions a picture standing alone in its paragraph with its alt text, still escaped', () => {
		const image =
			'<img src="/v1/guides/images/ab" alt="Your &quot;plan&quot; page" referrerpolicy="no-referrer" loading="lazy" />';
		const { html } = outline(`<p>${image}</p><p>Text ${image} inline</p>`);
		expect(html).toContain(
			`<figure class="gd-figure">${image}<figcaption>Your &quot;plan&quot; page</figcaption></figure>`
		);
		expect(html).toContain(`<p>Text ${image} inline</p>`);
	});
});
