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

/** What the editor holds, which is what a save sends.
 *
 * Publication is not one of these fields. A save writes the draft and nothing
 * else — publishing is its own request against a revision — so a status held
 * here would be a second, unauthoritative answer to "can a seller read this",
 * and a draft save that carried one could publish by accident. */
export interface GuideEdit {
	title: string;
	body: string;
	/** The one topic a guide sits under, by id, or none. */
	topic_id: string | null;
	/** Every tag it carries, by id. A set on the server, so order here is not
	 *  part of the edit. */
	tag_ids: string[];
}

/** Why this edit cannot be saved, or null where it can. The first refusal
 *  rather than all of them, because the Save control shows one reason.
 *
 * A topic and a tag are ids the server resolves; nothing here can tell a
 * retired id from a live one, so neither is refused locally. */
export function editRefusal(edit: GuideEdit): string | null {
	return titleRefusal(edit.title) ?? bodyRefusal(edit.body);
}

/** Whether two edits say the same thing. Sorted rather than positional for the
 *  tags, because the server stores a set: reordering the chips is not an edit
 *  to save, and treating it as one would make a save queue that never drains. */
export function sameEdit(left: GuideEdit, right: GuideEdit): boolean {
	if (
		left.title !== right.title ||
		left.body !== right.body ||
		left.topic_id !== right.topic_id ||
		left.tag_ids.length !== right.tag_ids.length
	) {
		return false;
	}
	const sorted = [...left.tag_ids].sort();
	const against = [...right.tag_ids].sort();
	return sorted.every((id, index) => id === against[index]);
}

/** Whether the buffer differs from what the server last answered. Field by
 *  field rather than by a revision counter: a revision moves on a publish that
 *  changed no text, and the operator's question is whether their typing is
 *  stored. */
export function dirty(saved: GuideEdit, edit: GuideEdit): boolean {
	return !sameEdit(saved, edit);
}

/** Where a guide image is read back from. The reader route, not the operator
 *  one: the Markdown this writes is what every seller's browser requests, and
 *  an `/admin` path would 404 for all of them. */
export function imageMarkdown(handle: string): string {
	return `![](/v1/guides/images/${handle})`;
}

/** What a reader is told about a picture a guide loads from another site.
 *
 * Stated rather than silent: that request is one the reader's browser makes to
 * that site, and the honest version of "we suppress the referrer" is saying
 * the request happens at all. The suppression itself is the renderer's — every
 * `<img>` it writes carries `referrerpolicy="no-referrer"`, and an address it
 * will not permit degrades to the alt text — so nothing here rewrites its
 * output. */
export const IMAGE_PRIVACY =
	'Some pictures in a guide are loaded from the site that hosts them. Your browser fetches those directly, without telling that site which page you are reading.';

/** Whether a rendering loads a picture from another site, which is the only
 *  case the disclosure above has anything to say about.
 *
 * Case-insensitive on the scheme, because the renderer permits a scheme
 * case-insensitively and writes the address back as the operator spelled it:
 * `![](HTTPS://host/a.png)` is an accepted external image, and a lowercase-only
 * test would load it while saying nothing. A site-relative `/v1/guides/images/…`
 * is this site's own and is not disclosed. */
export function loadsRemoteImages(html: string): boolean {
	return /<img\s[^>]*\bsrc="https:/i.test(html);
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

// ----------------------------------------------------------------- toolbar

/** The five insertions this toolbar adds to the shared formatting controls.
 *
 * Guides-specific and kept here rather than folded into `markUp`: that one is
 * the resource description's toolbar, its output travels to marketplaces as
 * `CopyFormat::Markdown`, and a heading or a footnote is not something any
 * marketplace description takes. The four it does own — bold, italic, bullets,
 * numbers — are still called through it, so the two toolbars cannot come to
 * write different Markdown for the same button. */
export type GuideMarkKind = 'heading' | 'subheading' | 'link' | 'image' | 'footnote';

/** The body after an insertion, and what stays selected.
 *
 * Same shape as `MarkedUp`, because the caret matters as much as the text: a
 * link whose URL is selected is one the operator finishes by typing, and a
 * heading that keeps its line selected can be undone by eye. */
export interface GuideMarked {
	text: string;
	start: number;
	end: number;
}

/** `##` and `###`, not `#`: the page's own heading is the h1 on every console
 *  screen, so a guide's top level is the level below it. No custom identifiers
 *  — what a heading is addressable as is the renderer's decision. */
const HEADING: Record<'heading' | 'subheading', string> = { heading: '## ', subheading: '### ' };

/** What a link and a picture are written with before the operator types the
 *  address. A bare scheme rather than an example host: it is selected, so what
 *  it has to be is a prefix worth typing over. */
const URL_STUB = 'https://';

const LABEL_STUB = 'link text';

/** Every footnote identifier the body already uses, reference or definition. */
export function footnoteIdentifiers(text: string): Set<string> {
	const used = new Set<string>();
	for (const found of text.matchAll(/\[\^([^\]\s]+)\]/g)) {
		const id = found[1];
		if (id !== undefined) {
			used.add(id);
		}
	}
	return used;
}

/** A footnote identifier this body does not already hold.
 *
 * Fresh rather than counted: the visible number is the renderer's, assigned by
 * first reference, so an identifier only has to be unique — and reusing one
 * would silently merge two notes into whichever the renderer saw first. */
export function nextFootnoteId(text: string): string {
	const used = footnoteIdentifiers(text);
	for (let n = 1; ; n += 1) {
		const id = `fn${n}`;
		if (!used.has(id)) {
			return id;
		}
	}
}

/** The body with one of this toolbar's insertions written into it.
 *
 * The selection is never thrown away: a heading keeps the lines it marked
 * selected, a link and a picture keep the selected words as the label and
 * select the address instead, and a footnote leaves the marked text alone and
 * puts the caret in the note it opened. */
export function markGuide(
	text: string,
	start: number,
	end: number,
	kind: GuideMarkKind
): GuideMarked {
	const from = Math.max(0, Math.min(start, text.length));
	const to = Math.max(from, Math.min(end, text.length));
	if (kind === 'heading' || kind === 'subheading') {
		// The whole lines the selection touches, because prefixing from the
		// middle of a line would put the hashes inside a sentence.
		const lineFrom = text.lastIndexOf('\n', Math.max(from - 1, 0)) + 1;
		const broken = text.indexOf('\n', to);
		const lineTo = broken === -1 ? text.length : broken;
		// The old level comes off first, so pressing Heading on a subheading
		// changes it rather than writing `## ### `.
		const marked = text
			.slice(lineFrom, lineTo)
			.split('\n')
			.map((line) => `${HEADING[kind]}${line.replace(/^#{1,6}[ \t]*/, '')}`)
			.join('\n');
		return {
			text: `${text.slice(0, lineFrom)}${marked}${text.slice(lineTo)}`,
			start: lineFrom,
			end: lineFrom + marked.length
		};
	}
	if (kind === 'footnote') {
		const id = nextFootnoteId(text);
		// The reference goes after what was selected rather than over it: a
		// footnote annotates a phrase, so replacing the phrase with the marker
		// would delete the thing being annotated.
		const referenced = `${text.slice(0, to)}[^${id}]${text.slice(to)}`;
		const separator = referenced.endsWith('\n\n') ? '' : referenced.endsWith('\n') ? '\n' : '\n\n';
		const written = `${referenced}${separator}[^${id}]: `;
		// The caret lands in the definition, which is the one part of a
		// footnote the operator still has to write.
		return { text: written, start: written.length, end: written.length };
	}
	const selected = text.slice(from, to);
	if (selected.length === 0 && kind === 'link') {
		const snippet = `[${LABEL_STUB}](${URL_STUB})`;
		return {
			text: `${text.slice(0, from)}${snippet}${text.slice(to)}`,
			start: from + 1,
			end: from + 1 + LABEL_STUB.length
		};
	}
	const bang = kind === 'image' ? '!' : '';
	const snippet = `${bang}[${selected}](${URL_STUB})`;
	// The address is what is selected: the label is either the words the
	// operator had selected or deliberately empty on a picture, and the URL is
	// the part they are about to paste.
	const urlAt = from + bang.length + selected.length + 3;
	return {
		text: `${text.slice(0, from)}${snippet}${text.slice(to)}`,
		start: urlAt,
		end: urlAt + URL_STUB.length
	};
}
