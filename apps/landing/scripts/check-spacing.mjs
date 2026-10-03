// Words that run into a link or a bold phrase: `email<a>…</a>` reads as
// "emailcontact@…", and `<a>Terms</a>and` as "Termsand". The source cannot
// show it reliably: Astro's compiler drops some line breaks between a tag and
// the text that follows it on the next line (`</a>` then `says` on a new line
// builds as `</a>says`) and keeps others, and an interpolation such as
// `{supportEmail}` carries no tag at all. So this reads the built HTML: every
// page under `dist`, as a reader's browser lays out its text.
//
// At each open or close tag of an inline element, the character before the
// tag and the character after it are found in the text flow, stepping over
// other inline tags and stopping at anything else (a block tag is a break). A
// letter or digit on both sides means two words were glued together, when
// bare text touches the tag on at least one side. Two sibling elements back to
// back (`</a><a>` in a nav, an icon `<span>` before a label) are laid out by
// the stylesheet as separate boxes, not run together as prose, so they pass.
//
// Run from `apps/landing` after `npm run build`: `just landing-spacing-gate`.

import { readdirSync, readFileSync } from 'node:fs';

const INLINE = new Set(['a', 'strong', 'em', 'code', 'span']);
// Elements whose content is not reading text.
const OPAQUE = new Set(['script', 'style', 'svg', 'template', 'noscript', 'title']);
const WORD = /[\p{L}\p{N}]/u;

const ENTITIES = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ' };
const decode = (text) =>
	text.replace(/&(#x[0-9a-f]+|#[0-9]+|[a-z]+);/gi, (whole, name) => {
		if (name[0] === '#') {
			const hex = name[1] === 'x' || name[1] === 'X';
			return String.fromCodePoint(Number.parseInt(name.slice(hex ? 2 : 1), hex ? 16 : 10));
		}
		// Any other named entity (`&rsquo;`, `&middot;`) is punctuation.
		return ENTITIES[name.toLowerCase()] ?? '.';
	});

/** Tags and text, in document order, with opaque elements' contents dropped. */
const tokenize = (html) => {
	const tokens = [];
	const tag = /<!--[\s\S]*?-->|<!doctype[^>]*>|<(\/?)([a-zA-Z][a-zA-Z0-9-]*)\b[^>]*?(\/?)>/gi;
	let at = 0;
	let skipping = null;
	for (let match = tag.exec(html); match !== null; match = tag.exec(html)) {
		const [whole, closing, rawName = '', selfClosing] = match;
		const name = rawName.toLowerCase();
		if (skipping !== null) {
			if (closing && name === skipping) {
				skipping = null;
				at = match.index + whole.length;
			}
			continue;
		}
		if (match.index > at) {
			tokens.push({ kind: 'text', text: decode(html.slice(at, match.index)), offset: at });
		}
		at = match.index + whole.length;
		if (name === '') {
			continue;
		}
		if (!closing && !selfClosing && OPAQUE.has(name)) {
			skipping = name;
			// An opaque element is a break in the flow, like a block.
			tokens.push({ kind: 'tag', name, inline: false, closing: false, offset: match.index });
			continue;
		}
		tokens.push({
			kind: 'tag',
			name,
			inline: INLINE.has(name),
			closing: closing === '/',
			offset: match.index
		});
	}
	if (at < html.length && skipping === null) {
		tokens.push({ kind: 'text', text: decode(html.slice(at)), offset: at });
	}
	return tokens;
};

/** The nearest character to one side of `index`, or null at a break. */
const neighbour = (tokens, index, step) => {
	for (let i = index + step; i >= 0 && i < tokens.length; i += step) {
		const token = tokens[i];
		if (token.kind === 'tag') {
			if (!token.inline) {
				return null;
			}
			continue;
		}
		if (token.text.length > 0) {
			return step < 0 ? token.text.at(-1) : token.text[0];
		}
	}
	return null;
};

const snippet = (html, offset) =>
	html
		.slice(Math.max(0, offset - 50), offset + 60)
		.replace(/\s+/g, ' ')
		.trim();

const files = (dir, suffix, into = []) => {
	for (const entry of readdirSync(dir, { withFileTypes: true })) {
		const path = `${dir}/${entry.name}`;
		if (entry.isDirectory()) {
			files(path, suffix, into);
		} else if (path.endsWith(suffix)) {
			into.push(path);
		}
	}
	return into.sort();
};
const pages = files('dist', '.html');

// A walk that found nothing would otherwise pass by having nothing to say.
if (pages.length === 0) {
	console.error('landing: no built pages under dist, so the spacing gate is reading nothing');
	process.exit(1);
}

const hits = [];
let boundaries = 0;
for (const page of pages) {
	const html = readFileSync(page, 'utf8');
	const tokens = tokenize(html);
	tokens.forEach((token, index) => {
		if (token.kind !== 'tag' || !token.inline) {
			return;
		}
		boundaries += 1;
		// The side of the tag outside the element: before an open tag, after a
		// close tag. Words inside one element touching its own tags are its
		// content, not a join.
		const outside = tokens[token.closing ? index + 1 : index - 1];
		if (outside?.kind !== 'text') {
			return;
		}
		const before = neighbour(tokens, index, -1);
		const after = neighbour(tokens, index, 1);
		if (before !== null && after !== null && WORD.test(before) && WORD.test(after)) {
			hits.push(
				`${page}: "${before}" and "${after}" touch at <${token.closing ? '/' : ''}${token.name}>: ${snippet(html, token.offset)}`
			);
		}
	});
}

// An interpolation leaves no tag in the built page, so `is` at the end of one
// line and `{amount}` at the start of the next, which Astro joins as "is7",
// is caught in the source instead. Astro drops a line break between text and
// an expression entirely, so any text at a line end against an expression at
// the next line's start is a join ("unused:8" as much as "is7"), and so is an
// expression at a line end against a word on the next line. Only the template
// is read, with scripts, styles and `{/* */}` comments blanked out, so
// JavaScript in the frontmatter or a `<script>` never matches.
const EXPRESSION_START = /^\{(?!\/\*|\.\.\.|\s*['"`]\s*['"`]\s*\})[^{}]+\}/;
// Text a line can end on: not a tag's `>`, and not the braces and brackets
// that open or close an expression or a `.map(` body.
const TEXT_END = /[^\s>{}()[\]]$/;
// An expression after `=` is an attribute value, after `$` part of a template
// literal, and a `{...spread}` a set of attributes, none of them text.
const EXPRESSION_END = /(?:^|[^=$])\{(?!\.\.\.)[^{}]+\}$/;
let joins = 0;
for (const source of files('src', '.astro')) {
	const text = readFileSync(source, 'utf8');
	const fence = text.startsWith('---') ? text.indexOf('\n---', 3) : -1;
	const template = (fence === -1 ? text : text.slice(fence + 4)).replace(
		/<script\b[\s\S]*?<\/script>|<style\b[\s\S]*?<\/style>|\{\/\*[\s\S]*?\*\/\}/g,
		(block) => block.replace(/[^\n]/g, ' ')
	);
	const startLine = fence === -1 ? 1 : text.slice(0, fence + 4).split('\n').length;
	const lines = template.split('\n').map((line) => line.trim());
	lines.forEach((line, index) => {
		const next = lines[index + 1];
		if (line === '' || next === undefined || next === '') {
			return;
		}
		joins += 1;
		const wordThenExpression = TEXT_END.test(line) && EXPRESSION_START.test(next);
		const expressionThenWord = EXPRESSION_END.test(line) && WORD.test(next[0]);
		if (wordThenExpression || expressionThenWord) {
			hits.push(
				`${source}:${startLine + index}: a word and an expression meet across a line break, which the build joins with no space: "${line.slice(-30)}" / "${next.slice(0, 30)}"`
			);
		}
	});
}

if (hits.length > 0) {
	console.error('landing: words run together with no space between them:');
	for (const hit of hits) {
		console.error('  ' + hit);
	}
	process.exit(1);
}
console.log(
	`landing: ${pages.length} pages, ${boundaries} inline tag boundaries and ${joins} source line joins, no words run together`
);
