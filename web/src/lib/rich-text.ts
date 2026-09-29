// The description in its two spellings, and the arithmetic between them.
//
// A resource's body is either Markdown or HTML (`CopyFormat`). The rich-text
// editor writes HTML, and only the eight elements every marketplace field
// renders the same way: p, br, strong, em, ul, ol, li and a[href]. Anything
// else a browser or a paste puts in the editor is reduced to that here, and
// the server reduces it again on write (`crates/tam-api/src/rich_text.rs`), so
// the list below and the server's are the same list.
//
// Pure string work with no DOM, so the rules run the same under vitest as in
// the browser, and a pasted fragment is never parsed by a live document that
// would load its images.

import type { CopyFormat } from '$lib/generated/vocab';

// ---------------------------------------------------------------- parsing

type Node = string | Element;

interface Element {
	name: string;
	href: string | null;
	/** An `img`'s address and words, as written; the allow-list decides. */
	src: string | null;
	alt: string | null;
	children: Node[];
}

/** Elements with no closing tag. */
const VOID: Record<string, true> = {
	br: true,
	img: true,
	hr: true,
	input: true,
	meta: true,
	link: true,
	wbr: true,
	source: true,
	area: true,
	col: true
};

/** Elements whose content is not text a reader sees, dropped whole. */
const OPAQUE: Record<string, true> = {
	script: true,
	style: true,
	template: true,
	noscript: true,
	iframe: true,
	object: true,
	textarea: true,
	title: true,
	head: true,
	svg: true,
	math: true,
	select: true
};

/** Elements that start a new block wherever they appear. */
const BLOCKS: Record<string, true> = {
	p: true,
	div: true,
	h1: true,
	h2: true,
	h3: true,
	h4: true,
	h5: true,
	h6: true,
	blockquote: true,
	pre: true,
	section: true,
	article: true,
	header: true,
	footer: true,
	main: true,
	aside: true,
	nav: true,
	figure: true,
	figcaption: true,
	table: true,
	thead: true,
	tbody: true,
	tfoot: true,
	tr: true,
	dl: true,
	dt: true,
	dd: true,
	address: true,
	center: true,
	details: true,
	summary: true,
	li: true,
	ul: true,
	ol: true,
	hr: true
};

const NAMED_ENTITIES: Record<string, string> = {
	amp: '&',
	lt: '<',
	gt: '>',
	quot: '"',
	apos: "'",
	nbsp: '\u00a0',
	ndash: '–',
	mdash: '—',
	hellip: '…',
	lsquo: '‘',
	rsquo: '’',
	ldquo: '“',
	rdquo: '”',
	bull: '•',
	middot: '·',
	copy: '©',
	reg: '®',
	trade: '™',
	pound: '£',
	euro: '€'
};

/** Named and numeric references decoded; an unknown one stays as written. */
export function decodeEntities(text: string): string {
	return text.replace(
		/&(#[xX][0-9a-fA-F]{1,6}|#[0-9]{1,7}|[a-zA-Z]{2,8});/g,
		(whole, body: string) => {
			if (body.startsWith('#')) {
				const code =
					body[1] === 'x' || body[1] === 'X'
						? parseInt(body.slice(2), 16)
						: parseInt(body.slice(1), 10);
				return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : whole;
			}
			return NAMED_ENTITIES[body] ?? whole;
		}
	);
}

function escapeText(text: string): string {
	return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function escapeAttribute(text: string): string {
	return escapeText(text).replace(/"/g, '&quot;');
}

/** A link a reader can follow and nothing else: web and mail addresses. A
 *  `javascript:` or `data:` address is dropped with its tag, keeping the
 *  words. */
export function safeHref(raw: string): string | null {
	const href = decodeEntities(raw).trim().replace(/\s/g, '%20');
	return /^(https?:\/\/|mailto:)/i.test(href) ? href : null;
}

function attributeOf(attributes: string, wanted: string): string | null {
	const pattern = /([^\s"'>/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g;
	for (const match of attributes.matchAll(pattern)) {
		if (match[1].toLowerCase() === wanted) {
			return match[2] ?? match[3] ?? match[4] ?? '';
		}
	}
	return null;
}

/** A forgiving tree: unknown closing tags are ignored, unclosed ones end
 *  with their parent, and a new paragraph or list item closes the open one
 *  the way a browser would. Text is decoded. */
function parse(html: string): Element {
	const root: Element = { name: '#root', href: null, src: null, alt: null, children: [] };
	const stack: Element[] = [root];
	const top = () => stack[stack.length - 1];
	const closeTo = (name: string, fence: readonly string[] = []) => {
		for (let at = stack.length - 1; at > 0; at -= 1) {
			if (stack[at].name === name) {
				stack.length = at;
				return;
			}
			if (fence.includes(stack[at].name)) {
				return;
			}
		}
	};
	let at = 0;
	while (at < html.length) {
		const lt = html.indexOf('<', at);
		if (lt === -1) {
			top().children.push(decodeEntities(html.slice(at)));
			break;
		}
		if (lt > at) {
			top().children.push(decodeEntities(html.slice(at, lt)));
		}
		if (html.startsWith('<!--', lt)) {
			const end = html.indexOf('-->', lt + 4);
			at = end === -1 ? html.length : end + 3;
			continue;
		}
		const opener = /^<(\/?)([a-zA-Z][a-zA-Z0-9-]*)/.exec(html.slice(lt, lt + 64));
		if (opener === null) {
			if (html[lt + 1] === '!' || html[lt + 1] === '?') {
				const end = html.indexOf('>', lt);
				at = end === -1 ? html.length : end + 1;
			} else {
				top().children.push('<');
				at = lt + 1;
			}
			continue;
		}
		// The tag runs to the first `>` outside a quoted attribute value.
		let end = lt + opener[0].length;
		let quote: string | null = null;
		for (; end < html.length; end += 1) {
			const c = html[end];
			if (quote !== null) {
				if (c === quote) quote = null;
			} else if (c === '"' || c === "'") {
				quote = c;
			} else if (c === '>') {
				break;
			}
		}
		if (end >= html.length) {
			// An unclosed tag swallows the rest: its text was never visible.
			break;
		}
		const name = opener[2].toLowerCase();
		const closing = opener[1] === '/';
		const attributes = html.slice(lt + opener[0].length, end);
		at = end + 1;
		if (closing) {
			closeTo(name, name === 'li' ? ['ul', 'ol'] : []);
			continue;
		}
		if (OPAQUE[name] === true) {
			const close = new RegExp(`</${name}\\s*>`, 'i').exec(html.slice(at));
			at = close === null ? html.length : at + close.index + close[0].length;
			continue;
		}
		if (BLOCKS[name] === true) {
			closeTo('p', ['li', 'ul', 'ol', 'blockquote', 'td', 'th']);
		}
		if (name === 'li') {
			closeTo('li', ['ul', 'ol']);
		}
		const element: Element = {
			name,
			href: name === 'a' ? attributeOf(attributes, 'href') : null,
			src: name === 'img' ? attributeOf(attributes, 'src') : null,
			alt: name === 'img' ? attributeOf(attributes, 'alt') : null,
			children: []
		};
		top().children.push(element);
		if (VOID[name] !== true && !attributes.trimEnd().endsWith('/')) {
			stack.push(element);
		}
	}
	return root;
}

// ------------------------------------------------------------- sanitising

/** What one body may hold beyond the eight elements every body keeps. The
 *  description holds nothing more; an email adds two heading levels and
 *  pictures from addresses it trusts. */
export interface AllowList {
	/** Keeps `h2` and `h3` as headings; any other heading is a paragraph. */
	headings: boolean;
	/** The address an `img` keeps, or null to drop it. Absent: no pictures. */
	image?: (src: string) => string | null;
}

const DESCRIPTION: AllowList = { headings: false };

const HEADINGS: Record<string, true> = {
	h1: true,
	h2: true,
	h3: true,
	h4: true,
	h5: true,
	h6: true
};

function collapse(text: string): string {
	return text.replace(/[\t\n\r\f ]+/g, ' ').replace(/\u00a0/g, ' ');
}

/** Whether a fragment of our own output holds any visible words. */
function hasText(html: string): boolean {
	return decodeEntities(html.replace(/<[^>]*>/g, '')).trim().length > 0;
}

/** Whether a fragment of our own output shows anything: words or a picture. */
function hasContent(html: string): boolean {
	return hasText(html) || html.includes('<img ');
}

/** Leading and trailing spaces and line breaks off a run of inline HTML,
 *  and the doubled spaces two adjacent text nodes leave. */
function tidyInline(html: string): string {
	return html
		.replace(/ {2,}/g, ' ')
		.replace(/ ?<br> ?/g, '<br>')
		.replace(/^(?: |<br>)+/, '')
		.replace(/(?: |<br>)+$/, '');
}

/** Inline content with the allow-list applied. A block met inside inline
 *  content (a `div` inside a `strong`) is read as a line break. */
function inline(node: Node, allow: AllowList): string {
	if (typeof node === 'string') {
		return escapeText(collapse(node));
	}
	const inner = () => node.children.map((child) => inline(child, allow)).join('');
	switch (node.name) {
		case 'br':
			return '<br>';
		case 'img': {
			const src =
				node.src === null || allow.image === undefined
					? null
					: allow.image(decodeEntities(node.src).trim());
			const alt = collapse(decodeEntities(node.alt ?? '')).trim();
			return src === null
				? ''
				: `<img src="${escapeAttribute(src)}" alt="${escapeAttribute(alt)}">`;
		}
		case 'strong':
		case 'b':
			return wrap('strong', inner());
		case 'em':
		case 'i':
			return wrap('em', inner());
		case 'a': {
			const href = node.href === null ? null : safeHref(node.href);
			const words = inner();
			return href === null || !hasContent(words)
				? words
				: `<a href="${escapeAttribute(href)}">${words}</a>`;
		}
		case 'td':
		case 'th':
			return ` ${inner()} `;
		default:
			return BLOCKS[node.name] === true ? `<br>${inner()}<br>` : inner();
	}
}

/** An inline element around its content, with the spaces at its edges moved
 *  outside it so a Markdown rendering of the same text still emphasises. */
function wrap(tag: 'strong' | 'em', content: string): string {
	if (!hasText(content)) {
		return content;
	}
	const lead = /^(?: |<br>)*/.exec(content)?.[0] ?? '';
	const trail = /(?: |<br>)*$/.exec(content)?.[0] ?? '';
	const middle = content.slice(lead.length, content.length - trail.length);
	return `${lead}<${tag}>${middle}</${tag}>${trail}`;
}

/** A sequence of block children as paragraphs, headings and lists. */
function blocks(children: readonly Node[], out: string[], allow: AllowList): void {
	let run = '';
	const flush = () => {
		for (const piece of run.split(/(?:<br>\s*){2,}/)) {
			const tidy = tidyInline(piece);
			if (hasContent(tidy)) {
				out.push(`<p>${tidy}</p>`);
			}
		}
		run = '';
	};
	for (const child of children) {
		if (typeof child !== 'string' && (child.name === 'ul' || child.name === 'ol')) {
			flush();
			const list = listOf(child, allow);
			if (list !== '') {
				out.push(list);
			}
		} else if (
			typeof child !== 'string' &&
			allow.headings &&
			(child.name === 'h2' || child.name === 'h3')
		) {
			flush();
			const words = tidyInline(child.children.map((node) => inline(node, allow)).join(''));
			if (hasContent(words)) {
				out.push(`<${child.name}>${words}</${child.name}>`);
			}
		} else if (typeof child !== 'string' && BLOCKS[child.name] === true) {
			flush();
			if (child.name === 'tr') {
				run = child.children.map((node) => inline(node, allow)).join('');
				flush();
			} else {
				blocks(child.children, out, allow);
			}
		} else {
			run += inline(child, allow);
		}
	}
	flush();
}

function listOf(list: Element, allow: AllowList): string {
	const items: string[] = [];
	let loose = '';
	const flushLoose = () => {
		const tidy = tidyInline(loose);
		if (hasContent(tidy)) {
			items.push(`<li>${tidy}</li>`);
		}
		loose = '';
	};
	for (const child of list.children) {
		if (typeof child !== 'string' && child.name === 'li') {
			flushLoose();
			const item = itemOf(child, allow);
			if (item !== '') {
				items.push(`<li>${item}</li>`);
			}
		} else if (typeof child !== 'string' && (child.name === 'ul' || child.name === 'ol')) {
			// A list directly inside a list is how a browser indents: it
			// belongs to the item before it.
			flushLoose();
			const nested = listOf(child, allow);
			if (nested === '') {
				continue;
			}
			const last = items.pop();
			items.push(
				last === undefined ? `<li>${nested}</li>` : last.replace(/<\/li>$/, `${nested}</li>`)
			);
		} else {
			loose += inline(child, allow);
		}
	}
	flushLoose();
	return items.length === 0 ? '' : `<${list.name}>${items.join('')}</${list.name}>`;
}

function itemOf(item: Element, allow: AllowList): string {
	let words = '';
	const nested: string[] = [];
	for (const child of item.children) {
		if (typeof child !== 'string' && (child.name === 'ul' || child.name === 'ol')) {
			const list = listOf(child, allow);
			if (list !== '') nested.push(list);
		} else if (typeof child !== 'string' && BLOCKS[child.name] === true) {
			// A paragraph inside an item is a loose list's; its lines stay.
			words += `<br>${child.children.map((node) => inline(node, allow)).join('')}<br>`;
		} else {
			words += inline(child, allow);
		}
	}
	const tidy = tidyInline(words).replace(/(?:<br>){2,}/g, '<br>');
	const text = hasContent(tidy) ? tidy : '';
	return text === '' && nested.length === 0 ? '' : text + nested.join('');
}

/** Any HTML reduced to one allow-list: the eight elements every body keeps
 *  plus what `allow` adds, no attributes but a web or mail `href` and an
 *  allowed picture's `src` and `alt`, text escaped, every paragraph and item
 *  holding something to see. Empty when nothing is written. */
export function reduceHtml(html: string, allow: AllowList): string {
	const out: string[] = [];
	blocks(parse(html).children, out, allow);
	return out.join('');
}

/** The description as the editor and the server both keep it: the eight
 *  allowed elements, no attributes but a web or mail `href`, text escaped,
 *  every paragraph and item holding words. Empty when nothing is written, so
 *  a cleared editor reads as an unanswered field rather than as `<p></p>`. */
export function sanitiseHtml(html: string): string {
	return reduceHtml(html, DESCRIPTION);
}

/** What a reader sees, without markup: the words a counter counts. */
export function htmlText(html: string): string {
	const collect = (node: Node): string =>
		typeof node === 'string' ? node : node.children.map(collect).join('');
	return collect(parse(sanitiseHtml(html))).replace(/\u00a0/g, ' ');
}

/** The number the description's counter shows: the characters a reader
 *  sees. Tags are formatting, not characters; Markdown's marks are what the
 *  seller typed, so they count, as they always have. */
export function descriptionLength(body: string, format: CopyFormat): number {
	return format === 'Html' ? htmlText(body).length : body.length;
}

/** What rich text cannot keep from an HTML body, in a teacher's words: the
 *  parts `sanitiseHtml` drops rather than rewrites. Empty for anything the
 *  editor wrote itself. */
export function htmlLoss(html: string): string[] {
	const lost = new Set<string>();
	const walk = (node: Node) => {
		if (typeof node === 'string') return;
		const name = node.name;
		if (name === 'img' || name === 'picture' || name === 'figure') lost.add('pictures');
		else if (name === 'table') lost.add('tables');
		else if (HEADINGS[name] === true) lost.add('headings');
		else if (name === 'blockquote') lost.add('quotes');
		else if (name === 'pre' || name === 'code') lost.add('code');
		else if (name === 'hr') lost.add('dividing lines');
		else if (['video', 'audio', 'embed'].includes(name)) lost.add('videos');
		else if (['u', 's', 'del', 'strike', 'sub', 'sup', 'mark', 'font'].includes(name))
			lost.add('other text styles');
		else if (name === 'a' && (node.href === null || safeHref(node.href) === null))
			lost.add('links that are not web addresses');
		node.children.forEach(walk);
	};
	walk(parse(html));
	for (const opaque of Object.keys(OPAQUE)) {
		if (new RegExp(`<${opaque}[\\s>/]`, 'i').test(html)) {
			if (opaque === 'iframe' || opaque === 'object') lost.add('videos');
			else if (opaque === 'svg') lost.add('pictures');
		}
	}
	return [...lost];
}

// ------------------------------------------------------- Markdown to HTML

/** A Markdown body as HTML, by the CommonMark rules the TPT projection
 *  renders it with (`body_as_html` in tam-marketplace-tpt), then through the
 *  allow-list. `lost` names what the allow-list will not carry, so a switch
 *  can say so before it happens. */
export function markdownToHtml(markdown: string): { html: string; lost: string[] } {
	const lost = new Set<string>();
	const lines = markdown.replace(/\r\n?/g, '\n').split('\n');
	const raw = blockHtml(lines, lost);
	return { html: sanitiseHtml(raw), lost: [...lost] };
}

const BULLET = /^( {0,3})([-*+])( {1,4}|\t|$)(.*)$/;
const ORDERED = /^( {0,3})(\d{1,9})([.)])( {1,4}|\t|$)(.*)$/;
const RULE = /^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/;
const HEADING = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$/;
const FENCE = /^ {0,3}(`{3,}|~{3,})/;
const QUOTE = /^ {0,3}> ?(.*)$/;
const TABLE_RULE = /^ {0,3}\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;

interface Marker {
	ordered: boolean;
	/** `-`, `*`, `+`, or `.`/`)` after a number: a change starts a new list. */
	kind: string;
	/** Where the item's content starts, which is the indent its own
	 *  continuation lines need. */
	offset: number;
	text: string;
}

function markerOf(line: string): Marker | null {
	const bullet = BULLET.exec(line);
	if (bullet !== null && !RULE.test(line)) {
		const gap = bullet[3] === '' ? 1 : bullet[3].length;
		return { ordered: false, kind: bullet[2], offset: bullet[1].length + 1 + gap, text: bullet[4] };
	}
	const ordered = ORDERED.exec(line);
	if (ordered !== null) {
		const gap = ordered[4] === '' ? 1 : ordered[4].length;
		return {
			ordered: true,
			kind: ordered[3],
			offset: ordered[1].length + ordered[2].length + 1 + gap,
			text: ordered[5]
		};
	}
	return null;
}

function indentOf(line: string): number {
	return /^ */.exec(line.replace(/\t/g, '    '))?.[0].length ?? 0;
}

function isBlank(line: string): boolean {
	return line.trim() === '';
}

/** Whether a line starts a block of its own rather than continuing a
 *  paragraph. */
function interrupts(line: string): boolean {
	return (
		RULE.test(line) ||
		HEADING.test(line) ||
		FENCE.test(line) ||
		QUOTE.test(line) ||
		(markerOf(line) !== null && !isBlank(markerOf(line)?.text ?? ''))
	);
}

function blockHtml(lines: readonly string[], lost: Set<string>): string {
	let out = '';
	let at = 0;
	while (at < lines.length) {
		const line = lines[at];
		if (isBlank(line)) {
			at += 1;
			continue;
		}
		if (RULE.test(line)) {
			lost.add('dividing lines');
			at += 1;
			continue;
		}
		const heading = HEADING.exec(line);
		if (heading !== null) {
			lost.add('headings');
			out += `<p>${inlineHtml(heading[2] ?? '', lost)}</p>`;
			at += 1;
			continue;
		}
		const fence = FENCE.exec(line);
		if (fence !== null) {
			lost.add('code');
			const code: string[] = [];
			at += 1;
			while (at < lines.length && !lines[at].trim().startsWith(fence[1])) {
				code.push(escapeText(lines[at]));
				at += 1;
			}
			at += 1;
			out += `<p>${code.join('<br>')}</p>`;
			continue;
		}
		if (QUOTE.test(line)) {
			lost.add('quotes');
			const quoted: string[] = [];
			while (at < lines.length && !isBlank(lines[at])) {
				quoted.push(QUOTE.exec(lines[at])?.[1] ?? lines[at]);
				at += 1;
			}
			out += blockHtml(quoted, lost);
			continue;
		}
		const marker = markerOf(line);
		if (marker !== null) {
			const list = listHtml(lines, at, lost);
			out += list.html;
			at = list.next;
			continue;
		}
		if (indentOf(line) >= 4) {
			lost.add('code');
		}
		const paragraph: string[] = [line];
		at += 1;
		while (at < lines.length && !isBlank(lines[at]) && !interrupts(lines[at])) {
			paragraph.push(lines[at]);
			at += 1;
		}
		// A pipe table is its header row, a rule row, and body rows.
		if (paragraph.length >= 2 && paragraph[0].includes('|') && TABLE_RULE.test(paragraph[1])) {
			lost.add('tables');
			const rows = [paragraph[0], ...paragraph.slice(2)].map((row) =>
				row
					.replace(/^\s*\|/, '')
					.replace(/\|\s*$/, '')
					.split('|')
					.map((cell) => inlineHtml(cell.trim(), lost))
					.join(' ')
			);
			out += rows.map((row) => `<p>${row}</p>`).join('');
			continue;
		}
		// A setext heading underlines its paragraph.
		if (paragraph.length >= 2 && /^ {0,3}(=+|-+)[ \t]*$/.test(paragraph[paragraph.length - 1])) {
			lost.add('headings');
			paragraph.pop();
		}
		out += `<p>${paragraphHtml(paragraph, lost)}</p>`;
	}
	return out;
}

/** Lines of one paragraph: a soft break is a space to a reader, and two
 *  trailing spaces or a trailing backslash is a hard one. */
function paragraphHtml(lines: readonly string[], lost: Set<string>): string {
	let text = '';
	lines.forEach((line, index) => {
		const last = index === lines.length - 1;
		const stripped = line.replace(/^[ \t]+/, '');
		if (!last && / {2,}$/.test(stripped)) {
			text += `${stripped.replace(/ +$/, '')}\u0001`;
		} else if (!last && /\\$/.test(stripped) && !/\\\\$/.test(stripped)) {
			text += `${stripped.slice(0, -1)}\u0001`;
		} else {
			text += last ? stripped.replace(/[ \t]+$/, '') : `${stripped.replace(/[ \t]+$/, '')}\n`;
		}
	});
	return inlineHtml(text, lost).replace(/\u0001/g, '<br>');
}

function listHtml(
	lines: readonly string[],
	start: number,
	lost: Set<string>
): { html: string; next: number } {
	const first = markerOf(lines[start]) as Marker;
	const items: string[] = [];
	let at = start;
	while (at < lines.length) {
		const marker = markerOf(lines[at]);
		if (marker === null || marker.ordered !== first.ordered || marker.kind !== first.kind) {
			break;
		}
		const body: string[] = [marker.text];
		at += 1;
		let blank = false;
		while (at < lines.length) {
			const line = lines[at];
			if (isBlank(line)) {
				blank = true;
				body.push('');
				at += 1;
				continue;
			}
			if (indentOf(line) >= marker.offset) {
				body.push(line.replace(/\t/g, '    ').slice(marker.offset));
				blank = false;
				at += 1;
				continue;
			}
			// A lazy continuation: an unindented line straight after the
			// item's own text still belongs to its paragraph.
			if (!blank && !interrupts(line)) {
				body.push(line);
				at += 1;
				continue;
			}
			break;
		}
		while (body.length > 0 && isBlank(body[body.length - 1])) {
			body.pop();
		}
		// Items are always written tight: what is a paragraph inside a loose
		// item is a line of its own inside a tight one.
		items.push(
			`<li>${blockHtml(body, lost)
				.replace(/<\/p><p>/g, '<br>')
				.replace(/<\/?p>/g, '')}</li>`
		);
		if (at < lines.length && isBlank(lines[at - 1] ?? '') && markerOf(lines[at]) === null) {
			break;
		}
	}
	const tag = first.ordered ? 'ol' : 'ul';
	return { html: `<${tag}>${items.join('')}</${tag}>`, next: at };
}

/** Markdown's inline marks as HTML. The pieces that must not be read as
 *  marks — escapes, code, links, raw tags — are set aside first and put
 *  back last. */
function inlineHtml(text: string, lost: Set<string>): string {
	const held: string[] = [];
	const hold = (html: string) => {
		held.push(html);
		return `\u0000${held.length - 1}\u0000`;
	};
	let work = text
		.replace(/\\([!-/:-@[-`{-~])/g, (_, c: string) => hold(escapeText(c)))
		.replace(/(`+)([\s\S]*?[^`])\1(?!`)/g, (_, _ticks, code: string) => {
			lost.add('code');
			return hold(escapeText(code.trim()));
		})
		.replace(/<(https?:\/\/[^\s<>]+|mailto:[^\s<>]+)>/gi, (_, url: string) =>
			hold(`<a href="${escapeAttribute(url)}">${escapeText(url)}</a>`)
		)
		.replace(/!\[([^\]]*)\]\(([^)\s]*)(?:\s+"[^"]*")?\)/g, () => {
			lost.add('pictures');
			return '';
		})
		.replace(/<\/?[a-zA-Z][^>]*>/g, (tag) => hold(tag));
	work = escapeText(work);
	work = work.replace(
		/\[([^\]]+)\]\(([^)\s]*)(?:\s+"[^"]*")?\)/g,
		(_, words: string, url: string) => {
			const href = safeHref(url.replace(/&amp;/g, '&'));
			return href === null ? words : `<a href="${escapeAttribute(href)}">${words}</a>`;
		}
	);
	work = work
		.replace(/\*\*\*(?=\S)([\s\S]*?\S)\*\*\*/g, '<em><strong>$1</strong></em>')
		.replace(/\*\*(?=\S)([\s\S]*?\S)\*\*/g, '<strong>$1</strong>')
		.replace(/(^|[^\p{L}\p{N}_])__(?=\S)([\s\S]*?\S)__(?![\p{L}\p{N}_])/gu, '$1<strong>$2</strong>')
		.replace(/\*(?=[^\s*])([\s\S]*?[^\s*])\*/g, '<em>$1</em>')
		.replace(/\*(?=[^\s*])([^*]?)\*/g, '<em>$1</em>')
		.replace(/(^|[^\p{L}\p{N}_])_(?=\S)([\s\S]*?\S)_(?![\p{L}\p{N}_])/gu, '$1<em>$2</em>')
		.replace(/~~(?=\S)([\s\S]*?\S)~~/g, (_, words: string) => {
			lost.add('other text styles');
			return words;
		});
	return work.replace(/\u0000(\d+)\u0000/g, (_, index: string) => held[Number(index)]);
}

// ------------------------------------------------------- HTML to Markdown

/** An HTML body as the Markdown the toolbar would have written: `**` and `*`,
 *  `-` and `1.` lists, `[words](address)` links, paragraphs a blank line
 *  apart. Anything outside the allow-list is gone first; `htmlLoss` names
 *  it. */
export function htmlToMarkdown(html: string): string {
	const root = parse(sanitiseHtml(html));
	return root.children
		.map((node) => (typeof node === 'string' ? escapeMarkdown(node) : blockMarkdown(node, '')))
		.join('\n\n');
}

function blockMarkdown(node: Element, indent: string): string {
	if (node.name === 'ul' || node.name === 'ol') {
		let number = 0;
		return node.children
			.filter((child): child is Element => typeof child !== 'string' && child.name === 'li')
			.map((item) => {
				number += 1;
				const marker = node.name === 'ul' ? '- ' : `${number}. `;
				const inner = indent + ' '.repeat(marker.length);
				const words = item.children
					.filter(
						(child) => typeof child === 'string' || (child.name !== 'ul' && child.name !== 'ol')
					)
					.map(inlineMarkdown)
					.join('')
					.replace(/\n/g, `\n${inner}`);
				const nested = item.children
					.filter(
						(child): child is Element =>
							typeof child !== 'string' && (child.name === 'ul' || child.name === 'ol')
					)
					.map((list) => `\n${blockMarkdown(list, inner)}`)
					.join('');
				return `${indent}${marker}${escapeLineStart(words)}${nested}`;
			})
			.join('\n');
	}
	return escapeLineStart(node.children.map(inlineMarkdown).join(''));
}

function inlineMarkdown(node: Node): string {
	if (typeof node === 'string') {
		return escapeMarkdown(node);
	}
	const inner = node.children.map(inlineMarkdown).join('');
	switch (node.name) {
		case 'br':
			return '  \n';
		case 'strong':
			return `**${inner}**`;
		case 'em':
			return `*${inner}*`;
		case 'a':
			return `[${inner}](${node.href ?? ''})`;
		default:
			return inner;
	}
}

function escapeMarkdown(text: string): string {
	return text.replace(/([\\`*_[\]<])/g, '\\$1').replace(/&(?=#?[a-zA-Z0-9]+;)/g, '\\&');
}

/** A line whose words happen to start like a heading, a quote or a list
 *  marker is escaped, so it stays words. */
function escapeLineStart(text: string): string {
	return text.replace(/(^|\n *)([#>+-])/g, '$1\\$2').replace(/(^|\n *)(\d+)([.)])/g, '$1$2\\$3');
}

// -------------------------------------------------------------- Tes text

/** Tes's draft-page limit, in its own counter's unit (UTF-16). */
export const TES_DESCRIPTION_LIMIT = 3000;

/** The description as the Tes projection sends it: plain text, paragraphs a
 *  blank line apart, a list item per line with its bullet or number, no
 *  emoji, within Tes's limit. The same rules as `tes_description` in
 *  tam-marketplace-tes, for the preview; the server's is the one that
 *  posts. */
export function tesText(body: string, format: CopyFormat): string {
	const text = format === 'Html' ? flatten(body) : body;
	return truncate(
		tidy([...text].filter((c) => !isEmoji(c.codePointAt(0) ?? 0)).join('')),
		TES_DESCRIPTION_LIMIT
	);
}

function flatten(html: string): string {
	let out = '';
	const lists: { ordered: boolean; count: number }[] = [];
	let rest = html;
	for (;;) {
		const lt = rest.indexOf('<');
		if (lt === -1) break;
		out += rest.slice(0, lt);
		const gt = rest.indexOf('>', lt);
		if (gt === -1) {
			rest = '';
			break;
		}
		const tag = rest.slice(lt + 1, gt);
		rest = rest.slice(gt + 1);
		const closing = tag.startsWith('/');
		const name = (/^\/?([a-zA-Z0-9]*)/.exec(tag)?.[1] ?? '').toLowerCase();
		if (name === 'li' && !closing) {
			const list = lists[lists.length - 1];
			if (list?.ordered) {
				list.count += 1;
				out += `\n${list.count}. `;
			} else {
				out += '\n- ';
			}
		} else if ((name === 'ul' || name === 'ol') && !closing) {
			lists.push({ ordered: name === 'ol', count: 0 });
		} else if ((name === 'ul' || name === 'ol') && closing) {
			lists.pop();
			// A nested list ends inside its item; only the outermost breaks.
			if (lists.length === 0) out += '\n';
		} else if ((name === 'li' || name === 'tr') && closing) {
			// An item ends where the next begins.
		} else if (name === 'table' && !closing) {
			// The rows break their own lines.
		} else if (
			[
				'table',
				'br',
				'p',
				'div',
				'h1',
				'h2',
				'h3',
				'h4',
				'h5',
				'h6',
				'blockquote',
				'pre',
				'hr',
				'section',
				'article',
				'header',
				'footer'
			].includes(name)
		) {
			out += '\n';
		} else if (name === 'td' || name === 'th') {
			out += ' ';
		}
	}
	out += rest;
	return decodeEntities(out);
}

function isEmoji(code: number): boolean {
	return (
		code === 0x200d ||
		code === 0x20e3 ||
		(code >= 0xfe0e && code <= 0xfe0f) ||
		(code >= 0x2600 && code <= 0x27bf) ||
		(code >= 0x2b00 && code <= 0x2bff) ||
		(code >= 0x1f000 && code <= 0x1faff) ||
		(code >= 0xe0020 && code <= 0xe007f)
	);
}

function tidy(text: string): string {
	let out = '';
	let blankPending = false;
	for (const line of text.split(/\r?\n/)) {
		const words = line.split(/\s+/).filter((word) => word !== '');
		if (words.length === 0) {
			blankPending = out !== '';
			continue;
		}
		if (blankPending) out += '\n\n';
		else if (out !== '') out += '\n';
		blankPending = false;
		out += words.join(' ');
	}
	return out;
}

function truncate(text: string, limit: number): string {
	if (text.length <= limit) return text;
	let cut = limit;
	// Never between the halves of a surrogate pair.
	if (/[\ud800-\udbff]/.test(text[cut - 1] ?? '')) cut -= 1;
	const head = text.slice(0, cut);
	const at = head.search(/\s\S*$/);
	return (at === -1 ? head : head.slice(0, at)).trimEnd();
}
