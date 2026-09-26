// The reader's side of Help and guides: the index grouped into its sections,
// and a published guide's HTML given an in-page contents and captioned
// pictures.
//
// The HTML is the server's own rendering (`crates/tam-api/src/guides.rs`),
// whose shapes are fixed: a heading is `<h2>…</h2>` with no attributes, and a
// picture is `<img src=… alt=… referrerpolicy=… loading=… />`, alone in its
// paragraph when the author wrote it on its own line. Only those two shapes
// are rewritten here, and the words of the guide are never touched.

import type { GuideHeadView, GuideTaxon } from '$lib/api';
import type { IconName } from '$lib/icons';

/** The five sections the guides are filed under, in the order a new seller
 *  meets them, by the topic's slug. */
export const SECTIONS: readonly { slug: string; icon: IconName }[] = [
	{ slug: 'getting-started', icon: 'sparkles' },
	{ slug: 'marketplaces', icon: 'store' },
	{ slug: 'catalogue', icon: 'library-big' },
	{ slug: 'automations', icon: 'workflow' },
	{ slug: 'billing', icon: 'credit-card' }
];

export interface GuideSection {
	/** The topic's id, or null for the guides filed under none. */
	id: string | null;
	/** The topic's slug, which is what places the section in `SECTIONS`. */
	slug: string | null;
	name: string;
	icon: IconName;
	guides: GuideHeadView[];
}

/** The guides on a page, grouped by topic: the five known sections first in
 *  their order, any other topic after them in the order it first appears, and
 *  the guides with no topic last. Each group keeps the order the server
 *  answered in, which is the order the reader chose. */
export function groupBySection(guides: readonly GuideHeadView[]): GuideSection[] {
	const groups = new Map<string | null, GuideSection>();
	for (const guide of guides) {
		const topic: GuideTaxon | null = guide.topic;
		const key = topic?.id ?? null;
		let group = groups.get(key);
		if (group === undefined) {
			group = {
				id: key,
				slug: topic?.slug ?? null,
				name: topic === null ? 'Other guides' : topic.name,
				icon: SECTIONS.find((section) => section.slug === topic?.slug)?.icon ?? 'book-open',
				guides: []
			};
			groups.set(key, group);
		}
		group.guides.push(guide);
	}
	const rank = (group: GuideSection) => {
		if (group.id === null) {
			return SECTIONS.length + 1;
		}
		const at = SECTIONS.findIndex((section) => section.slug === group.slug);
		return at === -1 ? SECTIONS.length : at;
	};
	// A stable sort, so two unknown topics keep the order they first appeared.
	return [...groups.values()].sort((left, right) => rank(left) - rank(right));
}

export interface Heading {
	id: string;
	text: string;
}

const ENTITIES: Record<string, string> = {
	'&amp;': '&',
	'&lt;': '<',
	'&gt;': '>',
	'&quot;': '"',
	'&#39;': "'"
};

/** A heading's words as a reader sees them: its markup dropped and the five
 *  escapes the renderer writes turned back into characters. */
function headingText(inner: string): string {
	return inner.replace(/<[^>]*>/g, '').replace(/&(amp|lt|gt|quot|#39);/g, (entity) => ENTITIES[entity]);
}

/** A heading's anchor: its words in lower case, joined by hyphens, and made
 *  unique on the page by a counter where two headings say the same thing. */
function anchorFor(text: string, taken: Set<string>): string {
	const base =
		text
			.toLowerCase()
			.replace(/[^a-z0-9]+/g, '-')
			.replace(/^-|-$/g, '') || 'section';
	let id = base;
	for (let n = 2; taken.has(id); n += 1) {
		id = `${base}-${n}`;
	}
	taken.add(id);
	return id;
}

/** A guide's HTML with an anchor on every second-level heading and every
 *  picture that stands alone in its paragraph drawn as a figure captioned by
 *  its alt text, and the headings in order for the in-page contents.
 *
 *  The caption is the alt text as the renderer already escaped it into the
 *  attribute, so it is placed as markup without being decoded: text that was
 *  safe inside an attribute value is safe as element content too. */
export function outline(html: string): { html: string; headings: Heading[] } {
	const headings: Heading[] = [];
	const taken = new Set<string>();
	const anchored = html.replace(/<h2>([\s\S]*?)<\/h2>/g, (_whole, inner: string) => {
		const text = headingText(inner);
		const id = anchorFor(text, taken);
		headings.push({ id, text });
		return `<h2 id="gd-${id}">${inner}</h2>`;
	});
	const figured = anchored.replace(
		/<p>\s*(<img [^>]*?alt="([^"]*)"[^>]*\/?>)\s*<\/p>/g,
		(_whole, image: string, alt: string) =>
			alt.length === 0
				? `<figure class="gd-figure">${image}</figure>`
				: `<figure class="gd-figure">${image}<figcaption>${alt}</figcaption></figure>`
	);
	return {
		html: figured,
		headings: headings.map((heading) => ({ ...heading, id: `gd-${heading.id}` }))
	};
}

/** The guide after this one in the index's order — by section, then as the
 *  server listed them — or null for the last guide and for a slug the list
 *  does not hold. */
export function nextGuide(
	guides: readonly GuideHeadView[],
	slug: string
): GuideHeadView | null {
	const ordered = groupBySection(guides).flatMap((section) => section.guides);
	const at = ordered.findIndex((guide) => guide.slug === slug);
	return at === -1 ? null : (ordered[at + 1] ?? null);
}
