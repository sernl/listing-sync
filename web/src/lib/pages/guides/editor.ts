// The guide editor's own decisions, none of which need a component: what a
// slug, a title and a body have to be before the server will take them, where
// an image's Markdown goes, and whether what is on screen still matches what
// was saved.
//
// The bounds below are the ones `guide` is declared with, restated here so the
// editor refuses a save the server would refuse rather than sending it and
// rendering a 422. A bound that drifts apart from the migration is a form that
// accepts what the table rejects, which is why each one names the constraint
// it mirrors.

import type { GuideStatus } from '$lib/api';

/** A slug is 1..80 characters of lowercase kebab, matching the `guide.slug`
 *  check constraint. It is the guide's address, so it is the one field the
 *  editor cannot change after creation without breaking every link to it. */
export const SLUG_MAX = 80;
export const TITLE_MAX = 120;

/** 200 KiB of Markdown, counted in bytes rather than characters because the
 *  column is bounded in bytes and an emoji is four of them. */
export const BODY_MAX_BYTES = 200 * 1024;

const SLUG_SHAPE = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

const ENCODER = new TextEncoder();

/** A title turned into the slug the operator would have typed.
 *
 * Offered as the initial value when a guide is created and never applied
 * afterwards: a slug that followed its title around would change a guide's
 * address every time somebody fixed a typo in the heading. */
export function slugify(title: string): string {
	return title
		.toLowerCase()
		.normalize('NFKD')
		.replace(/[^a-z0-9]+/g, '-')
		.replace(/^-+|-+$/g, '')
		.slice(0, SLUG_MAX)
		.replace(/-+$/, '');
}

/** Why this slug cannot be used, or null where it can. */
export function slugRefusal(slug: string): string | null {
	if (slug.length === 0) {
		return 'A guide needs a slug: it is the address sellers reach it at.';
	}
	if (slug.length > SLUG_MAX) {
		return `A slug is at most ${SLUG_MAX} characters.`;
	}
	if (!SLUG_SHAPE.test(slug)) {
		return 'A slug is lowercase letters, digits and single hyphens — no spaces, no punctuation, and no hyphen at either end.';
	}
	return null;
}

/** Why this title cannot be saved, or null where it can. */
export function titleRefusal(title: string): string | null {
	if (title.trim().length === 0) {
		return 'A guide needs a title.';
	}
	return title.length > TITLE_MAX ? `A title is at most ${TITLE_MAX} characters.` : null;
}

/** Why this body cannot be saved, or null where it can. Empty is allowed: a
 *  draft with a title and nothing under it is how a guide starts. */
export function bodyRefusal(body: string): string | null {
	const bytes = ENCODER.encode(body).length;
	return bytes > BODY_MAX_BYTES
		? `A guide body is at most ${Math.round(BODY_MAX_BYTES / 1024)} KiB; this one is ${Math.round(bytes / 1024)} KiB.`
		: null;
}

/** What the editor holds, which is what a save sends. */
export interface GuideDraft {
	title: string;
	body: string;
	status: GuideStatus;
}

/** Why this draft cannot be saved, or null where it can. The first refusal
 *  rather than all of them, because the Save control shows one reason. */
export function draftRefusal(draft: GuideDraft): string | null {
	return titleRefusal(draft.title) ?? bodyRefusal(draft.body);
}

/** Whether the draft differs from what was last saved. Field by field rather
 *  than by a revision counter: the server answers the stored guide back on
 *  every save, so the saved copy is always the server's own. */
export function dirty(saved: GuideDraft, draft: GuideDraft): boolean {
	return (
		saved.title !== draft.title || saved.body !== draft.body || saved.status !== draft.status
	);
}

/** Where a guide image is read back from. The reader route, not the operator
 *  one: the Markdown this writes is what every seller's browser requests, and
 *  an `/admin` path would 404 for all of them. */
export function imageMarkdown(handle: string): string {
	return `![](/v1/guides/images/${handle})`;
}

/** The text with a snippet put at the caret, and where the caret goes next.
 *
 * A selection is replaced rather than wrapped, which is what a picture insert
 * means; the caret lands after what was inserted so the operator keeps typing
 * in the place they were looking. */
export function insertAt(
	text: string,
	start: number,
	end: number,
	snippet: string
): { text: string; caret: number } {
	const from = Math.max(0, Math.min(start, text.length));
	const to = Math.max(from, Math.min(end, text.length));
	return { text: `${text.slice(0, from)}${snippet}${text.slice(to)}`, caret: from + snippet.length };
}
