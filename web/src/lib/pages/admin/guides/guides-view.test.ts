import { describe, expect, it } from 'vitest';
import type { GuideHeadView, GuideTaxon } from '$lib/api';
import { SETTLED, type Pipeline, type Write } from '$lib/pages/guides/save';
import {
	emptyGridMessage,
	publishPhase,
	publishedBehind,
	saveWords,
	statusCounts,
	visibleGuides,
	writeBlock,
	writeLabel
} from './guides-view';

const taxon = (id: string, name: string): GuideTaxon => ({
	id,
	slug: id,
	name,
	retired: false
});

function head(slug: string, over: Partial<GuideHeadView> = {}): GuideHeadView {
	return {
		id: `id-${slug}`,
		slug,
		title: slug,
		status: 'draft',
		updated_at: 0,
		revision: 1,
		topic: null,
		tags: [],
		...over
	};
}

const rows = [
	head('connect-etsy', {
		title: 'Connecting Etsy',
		status: 'published',
		updated_at: 300,
		topic: taxon('t1', 'Getting started')
	}),
	head('pricing', {
		title: 'pricing your work',
		updated_at: 100,
		tags: [taxon('g1', 'Etsy'), taxon('g2', 'Money')]
	}),
	head('bulk-edit', {
		title: 'Bulk edits',
		status: 'published',
		updated_at: 300
	})
];

const slugs = (list: GuideHeadView[]) => list.map((guide) => guide.slug);

describe('the guides grid', () => {
	it('counts every guide under its chip, whatever the search', () => {
		expect(statusCounts(rows)).toEqual({ all: 3, published: 2, draft: 1 });
	});

	it('keeps only the chosen status', () => {
		expect(slugs(visibleGuides(rows, 'draft', '', 'title'))).toEqual(['pricing']);
		expect(slugs(visibleGuides(rows, 'published', '', 'title'))).toEqual([
			'bulk-edit',
			'connect-etsy'
		]);
	});

	it('finds a guide by its topic, a tag or its address, ignoring case', () => {
		expect(slugs(visibleGuides(rows, 'all', 'GETTING', 'title'))).toEqual(['connect-etsy']);
		expect(slugs(visibleGuides(rows, 'all', 'money', 'title'))).toEqual(['pricing']);
		expect(slugs(visibleGuides(rows, 'all', 'bulk-ed', 'title'))).toEqual(['bulk-edit']);
	});

	it('narrows on every word of the search rather than any', () => {
		expect(slugs(visibleGuides(rows, 'all', 'etsy', 'title'))).toEqual(['connect-etsy', 'pricing']);
		expect(slugs(visibleGuides(rows, 'all', ' etsy   money ', 'title'))).toEqual(['pricing']);
	});

	it('orders by title without regard to case, or newest first with ties by title', () => {
		expect(slugs(visibleGuides(rows, 'all', '', 'title'))).toEqual([
			'bulk-edit',
			'connect-etsy',
			'pricing'
		]);
		expect(slugs(visibleGuides(rows, 'all', '', 'updated'))).toEqual([
			'bulk-edit',
			'connect-etsy',
			'pricing'
		]);
	});

	it('leaves the list it was given in its own order', () => {
		const before = slugs(rows);
		visibleGuides(rows, 'all', '', 'updated');
		expect(slugs(rows)).toEqual(before);
	});

	it('says a search found nothing before it says a filter is empty', () => {
		expect(emptyGridMessage('draft', 'zebra')).toBe('No guides match that search.');
		expect(emptyGridMessage('draft', '   ')).toBe('No drafts. Every guide is published.');
		expect(emptyGridMessage('published', '')).toBe('Nothing is published yet.');
	});
});

describe('where publication stands', () => {
	const saved = {
		title: 'Connecting Etsy',
		body: 'Open the shop.',
		topic: taxon('t1', 'Getting started'),
		tags: [taxon('g1', 'Etsy')]
	};
	const snapshot = {
		...saved,
		html: '<p>Open the shop.</p>',
		source_revision: 4,
		published_at: 10
	};

	it('is current when the published copy says what the draft says', () => {
		expect(publishedBehind({ ...saved, status: 'published', published: snapshot })).toBe(false);
	});

	it('is behind when the draft has changed text, topic or tags since publishing', () => {
		expect(
			publishedBehind({
				...saved,
				body: 'Open it.',
				status: 'published',
				published: snapshot
			})
		).toBe(true);
		expect(
			publishedBehind({
				...saved,
				topic: null,
				status: 'published',
				published: snapshot
			})
		).toBe(true);
		expect(
			publishedBehind({
				...saved,
				tags: [],
				status: 'published',
				published: snapshot
			})
		).toBe(true);
	});

	it('is never behind for a draft, which sellers cannot read at all', () => {
		expect(
			publishedBehind({
				...saved,
				body: 'Other',
				status: 'draft',
				published: snapshot
			})
		).toBe(false);
		expect(publishPhase('draft', true)).toBe('draft');
	});
});

describe('the save state and the write buttons', () => {
	const write: Write = {
		kind: 'save',
		expected_id: 'a',
		expected_revision: 1,
		edit: null
	};
	const halted: Pipeline = {
		flight: null,
		halt: { why: 'unreachable', write }
	};
	const flying: Pipeline = { flight: write, halt: null };

	it('reports a stopped write over one in flight over unsaved text', () => {
		expect(saveWords({ halt: halted.halt, busy: true, unsaved: true }).label).toBe('Not saved');
		expect(saveWords({ halt: null, busy: true, unsaved: true }).label).toBe('Saving');
		expect(saveWords({ halt: null, busy: false, unsaved: true }).label).toBe('Unsaved changes');
		expect(saveWords({ halt: null, busy: false, unsaved: false }).label).toBe('Saved');
	});

	it('lets a save run only with new text the server would take', () => {
		expect(writeBlock('save', { refusal: null, pipeline: SETTLED, unsaved: true })).toBeUndefined();
		expect(writeBlock('save', { refusal: null, pipeline: SETTLED, unsaved: false })).toBe(
			'No changes to save.'
		);
		expect(
			writeBlock('save', {
				refusal: 'Give it a title.',
				pipeline: halted,
				unsaved: true
			})
		).toBe('Give it a title.');
	});

	it('holds a publication until the text is saved, but not for a refused draft', () => {
		expect(
			writeBlock('publish', {
				refusal: 'x',
				pipeline: SETTLED,
				unsaved: false
			})
		).toBeUndefined();
		expect(
			writeBlock('publish', {
				refusal: null,
				pipeline: SETTLED,
				unsaved: true
			})
		).toBe('Save first. Publishing uses the saved version.');
		expect(
			writeBlock('unpublish', {
				refusal: null,
				pipeline: SETTLED,
				unsaved: true
			})
		).toBe('Save first, then unpublish.');
	});

	it('holds every write while another is out or a message is waiting', () => {
		for (const kind of ['save', 'publish', 'unpublish'] as const) {
			expect(writeBlock(kind, { refusal: null, pipeline: halted, unsaved: true })).toBe(
				'Resolve the message above first.'
			);
			expect(writeBlock(kind, { refusal: null, pipeline: flying, unsaved: false })).toBe(
				'Wait for the current change to finish.'
			);
		}
	});

	it('names a republish differently from a first publish, and shows which write is going', () => {
		expect(writeLabel('publish', 'draft', null)).toBe('Publish');
		expect(writeLabel('publish', 'behind', null)).toBe('Publish this version');
		expect(writeLabel('publish', 'live', 'publish')).toBe('Publishing…');
		expect(writeLabel('save', 'live', 'publish')).toBe('Save');
	});
});
