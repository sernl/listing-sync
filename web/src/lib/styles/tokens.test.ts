// The palette's own gate: contrast, drift between the sheets that declare it,
// and the absence of any colour this file does not name.
//
// It exists because the palette is the one thing in this console that is
// correct by arithmetic rather than by looking right, and arithmetic is exactly
// what a later edit made for looks will quietly break. Every value below is
// read from `tokens.css` rather than restated here, so nudging a token one
// shade fails the lane instead of shipping.

import { readFileSync, readdirSync, existsSync, statSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const HERE = new URL('.', import.meta.url).pathname;
const WEB_SRC = new URL('../../', import.meta.url).pathname;
const REPO = new URL('../../../../', import.meta.url).pathname;
const LANDING_CSS = `${REPO}apps/landing/src/styles/site.css`;
const STATIC = `${REPO}web/static`;
const OFFLINE = `${STATIC}/unreachable.html`;
const FAVICON = `${STATIC}/favicon.svg`;

/** The body of the first `:root` block of a stylesheet, comments dropped. */
function rootBlock(path: string): string {
	const sheet = readFileSync(path, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
	const block = sheet.slice(sheet.indexOf(':root'));
	return block.slice(block.indexOf('{') + 1, block.indexOf('}'));
}

/** Every `--name: value` inside the first `:root` block of a stylesheet. */
function rootTokens(path: string): Map<string, string> {
	const body = rootBlock(path);
	return new Map([...body.matchAll(/--([a-z0-9-]+)\s*:\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));
}

const TOKENS = rootTokens(`${HERE}tokens.css`);

/** A token's colour, following one level of `var()` aliasing. */
function colour(name: string): string {
	const raw = TOKENS.get(name);
	if (raw === undefined) throw new Error(`tokens.css declares no --${name}`);
	const alias = raw.match(/^var\(--([a-z0-9-]+)\)$/);
	return alias ? colour(alias[1]) : raw;
}

function luminance(hex: string): number {
	const h = hex.replace('#', '');
	const full = h.length === 3 ? [...h].map((c) => c + c).join('') : h;
	const channel = (i: number) => {
		const v = parseInt(full.slice(i, i + 2), 16) / 255;
		return v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
	};
	return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4);
}

/** WCAG 2.x contrast between two token names. */
function ratio(ink: string, ground: string): number {
	const a = luminance(colour(ink));
	const b = luminance(colour(ground));
	return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

/** The bar for every pair here.
 *
 *  4.5 rather than the 3.0 large-text allowance, with no exception for the
 *  status badges: a badge in this console is 10.5px uppercase, which is smaller
 *  than body text rather than larger, so nothing on screen earns the
 *  allowance. */
const AA = 4.5;

/** Every neutral a page is painted with, which is what body text sits on.
 *
 *  A saturated fill such as `--accent` is a ground too, but only under the ink
 *  the pairs below name for it, so it is checked there rather than swept in
 *  here. */
const GROUNDS = ['ground', 'surface', 'nav', 'hover', 'tint', 'tint-edge'];

/** Every token drawn as text somewhere in the console.
 *
 *  `inkTokensInUse` below asserts this list is still complete, so a token that
 *  starts being drawn as text fails the lane until somebody decides whether it
 *  clears the bar rather than discovering later that it never did. */
const INKS = [
	'text',
	'ink',
	'muted',
	'faint',
	'primary',
	'accent',
	'accent-ink',
	'accent-deep',
	'additive',
	'ok',
	'warn',
	'bad',
	'soon'
];

/** A badge's ink against the soft ground that badge is painted on. */
const ON_SOFT: Array<[string, string]> = [
	['accent-ink', 'accent-soft'],
	['additive', 'additive-soft'],
	['ok', 'ok-soft'],
	['warn', 'warn-soft'],
	['bad', 'bad-soft'],
	['soon', 'soon-soft']
];

/** A filled control's label against its own fill. */
const ON_FILL: Array<[string, string]> = [
	['on-fill', 'accent'],
	['on-fill', 'primary'],
	['on-fill', 'additive'],
	['on-fill', 'bad'],
	['on-fill', 'text'],
	['surface', 'primary']
];

/** The seven label hues, which are the one scale that does not follow the
 *  theme and are therefore checked on their own terms.
 *
 *  Both grounds, and `--ground` is the one that binds: `LabelsDialog.svelte`
 *  paints `.chip` with `background: var(--ground)` while the card behind it is
 *  the lighter `--surface`, so measuring the card alone passes a hue the chip
 *  fails. That is not hypothetical -- `--label-teal` measured 4.60 on the card
 *  and 4.26 on the chip, and only the second is what a seller reads. */
const LABELS = [...TOKENS.keys()].filter((name) => name.startsWith('label-'));

/** Every file under a directory whose name ends in one of these. */
function filesUnder(dir: string, endings: string[]): string[] {
	const out: string[] = [];
	for (const entry of readdirSync(dir)) {
		const path = `${dir}/${entry}`;
		if (entry === '.svelte-kit' || entry === 'node_modules') continue;
		if (statSync(path).isDirectory()) out.push(...filesUnder(path, endings));
		else if (endings.some((e) => entry.endsWith(e))) out.push(path);
	}
	return out;
}

/** `#fff` and `#ffffff` are one colour, and the sweep below compares by value.
 *  Anything that is not a hex is returned lowercased and otherwise untouched,
 *  because `TOKENS` holds measures and font stacks as well as colours. */
function expand(value: string): string {
	const written = value.toLowerCase();
	return /^#[0-9a-f]{3}$/.test(written)
		? `#${[...written.slice(1)].map((c) => c + c).join('')}`
		: written;
}

/** Directories under `web/static` this palette does not reach, each with the
 *  reason it is a decision rather than drift. */
const STATIC_DIRS: Record<string, string> = {
	'marketplaces/':
		"Each marketplace's own logo, drawn in colours that are the marketplace's and not ours to move.",
	'email/':
		'Two of the drawings here, `teachouse-mark.svg` and `teachouse-delivery.svg`, are still in the retired Kauri palette: a mail client composes from them rather than from `tokens.css`, so recolouring them is a founder decision that has not been taken. `teachouse-mark-small.svg` is already on the current palette, because `favicon-16.png` is rendered from it, and is inside the exception only because it lives beside the other two.'
};

/** The colours the mark is drawn in that no token names, so a palette edit
 *  leaves them where they are. */
const MARK_OWN: Record<string, string> = {
	'#f7f2e9': "The mark's light: the gable and the near page of the open book.",
	'#f2e9da': "The shade on the book's far page."
};

/** Everything else `web/static` writes that is neither a token's value nor the
 *  mark's own. */
const STATIC_OTHER: Record<string, string> = {
	'rgba(':
		"The offline card's shadow, which is `--sh-2` written out: `color-mix` is what the token uses and the webview the desktop app bundles may predate it. The channels are checked against `--text` below."
};

/** Every colour literal a file writes, comments dropped. */
function literalsIn(path: string): string[] {
	// Comments go first: a hex inside one is prose about the palette, not a
	// colour the browser will paint, and this file's own header says
	// `color: #fff` while explaining why none may remain.
	const body = readFileSync(path, 'utf8')
		.replace(/\/\*[\s\S]*?\*\//g, '')
		.replace(/<!--[\s\S]*?-->/g, '');
	const sheet =
		path.endsWith('tokens.css') || path === LANDING_CSS
			? body.replace(/:root\s*\{[\s\S]*?\n\}/, '')
			: body;
	return [...sheet.matchAll(/#[0-9a-fA-F]{3}\b|#[0-9a-fA-F]{6}\b|\brgba?\(|\bhsla?\(/g)].map(
		([literal]) => literal
	);
}

describe('contrast', () => {
	it.each(INKS)('--%s clears AA on every ground a page is painted with', (ink) => {
		for (const ground of GROUNDS) {
			expect(`--${ink} on --${ground}: ${ratio(ink, ground).toFixed(2)}`).toBe(
				`--${ink} on --${ground}: ${Math.max(ratio(ink, ground), AA).toFixed(2)}`
			);
		}
	});

	it.each(ON_SOFT)('--%s clears AA on --%s', (ink, ground) => {
		expect(ratio(ink, ground)).toBeGreaterThanOrEqual(AA);
	});

	it.each(ON_FILL)('--%s clears AA on --%s', (ink, ground) => {
		expect(ratio(ink, ground)).toBeGreaterThanOrEqual(AA);
	});

	it('every label hue clears AA on the chip and on the card', () => {
		expect(LABELS.length).toBe(7);
		const failing = LABELS.flatMap((hue) =>
			(['ground', 'surface'] as const)
				.filter((ground) => ratio(hue, ground) < AA)
				.map((ground) => `--${hue} on --${ground}: ${ratio(hue, ground).toFixed(2)}`)
		);
		expect(failing).toEqual([]);
	});

	it('the label hues stay seven distinct colours', () => {
		expect(new Set(LABELS.map(colour)).size).toBe(LABELS.length);
	});

	it('every token drawn as text is one the pairs above measure', () => {
		const drawn = new Set<string>();
		for (const file of filesUnder(WEB_SRC, ['.css', '.svelte'])) {
			for (const [, name] of readFileSync(file, 'utf8').matchAll(
				/(?<!-)\bcolor:\s*var\(--([a-z0-9-]+)\)/g
			)) {
				drawn.add(name);
			}
		}
		// A ground is legitimately drawn as text where it sits on a filled
		// control, and `ON_FILL` measures each of those.
		const measured = new Set([...INKS, ...ON_FILL.map(([ink]) => ink), ...LABELS]);
		expect([...drawn].filter((name) => !measured.has(name)).sort()).toEqual([]);
	});
});

/** Tokens the landing deliberately declares differently from the console, each
 *  with the reason it is a decision rather than drift.
 *
 *  The assertions below refuse an entry that has stopped differing, so an
 *  exception cannot outlive the reason it was granted for. */
const LANDING_OWN: Record<string, string> = {
	'r-card':
		"The marketing site's shape language is heyretro's rather than the console's: 2rem section cards against the console's 16px, which is a deliberate difference recorded in landing-page.md."
};

describe('one palette, declared once', () => {
	const landing = rootTokens(LANDING_CSS);
	const shared = [...landing.keys()].filter((name) => TOKENS.has(name));

	it('the landing and the console agree on every token both declare', () => {
		expect(shared.length).toBeGreaterThan(10);
		const drifted = shared
			.filter((name) => !(name in LANDING_OWN))
			.filter((name) => landing.get(name) !== colour(name))
			.map((name) => `--${name}: landing ${landing.get(name)}, console ${colour(name)}`);
		expect(drifted).toEqual([]);
	});

	it('every deliberate difference is still shared and still a difference', () => {
		const spent = Object.keys(LANDING_OWN).filter(
			(name) => !shared.includes(name) || landing.get(name) === colour(name)
		);
		expect(spent).toEqual([]);
	});

	/** Light-only is a decision with dark deferred, not an omission, and the
	 *  contrast pairs above are measured for light alone. A sheet that stops
	 *  declaring it hands form controls, scrollbars and the canvas to whatever
	 *  theme the reader's browser is set to. */
	it('both sheets declare the light scheme', () => {
		const missing = [`${HERE}tokens.css`, LANDING_CSS]
			.filter((path) => !/\bcolor-scheme:\s*light\s*;/.test(rootBlock(path)))
			.map((path) => path.slice(REPO.length));
		expect(missing).toEqual([]);
	});

	/** The sheets and templates that must reach every colour through a token,
	 *  and `web/static`, which cannot: the desktop app opens `unreachable.html`
	 *  from its own bundle with no stylesheet and no network, and an SVG has no
	 *  tokens to read, so both spell their colours out. A literal there is
	 *  allowed only where it is a token's own value -- a copy the palette still
	 *  owns, and one that stops matching the moment a token moves -- or where it
	 *  is named above. */
	const swept = () => [
		...filesUnder(WEB_SRC, ['.css', '.svelte']),
		...filesUnder(`${REPO}apps/landing/src`, ['.css', '.svelte', '.astro']),
		...filesUnder(STATIC, ['.html', '.svg']).filter(
			(file) =>
				!Object.keys(STATIC_DIRS).some((dir) => file.startsWith(`${STATIC}/${dir}`))
		)
	];

	const declared = new Set([...TOKENS.keys()].map((name) => expand(colour(name))));

	it('every colour on screen is a token, by var() or by a checked copy', () => {
		const stray: string[] = [];
		for (const file of swept()) {
			for (const literal of literalsIn(file)) {
				const written = expand(literal);
				const allowed =
					file.startsWith(`${STATIC}/`) &&
					(declared.has(written) || written in MARK_OWN || written in STATIC_OTHER);
				if (!allowed) {
					stray.push(`${file.slice(REPO.length)}: ${literal}`);
				}
			}
		}
		expect(stray).toEqual([]);
	});

	it('every colour web/static is allowed to write is still one it writes', () => {
		const written = new Set(
			swept()
				.filter((file) => file.startsWith(`${STATIC}/`))
				.flatMap((file) => literalsIn(file).map(expand))
		);
		const all = filesUnder(STATIC, ['.html', '.svg']);
		const spent = [
			...Object.keys(STATIC_DIRS).filter(
				(dir) =>
					!all.some((file) => file.startsWith(`${STATIC}/${dir}`) && literalsIn(file).length > 0)
			),
			// A token that has taken one of the mark's colours retires the
			// exception: the palette now owns it, and the sweep passes it.
			...Object.keys(MARK_OWN).filter((hex) => !written.has(hex) || declared.has(hex)),
			...Object.keys(STATIC_OTHER).filter((literal) => !written.has(literal))
		];
		expect(spent).toEqual([]);
	});
});

/** The console's palette reaches the desktop app's offline page as a copy,
 *  because that page is opened from the app's own bundle when the server
 *  cannot be reached and so has no stylesheet to import. The copy is checked
 *  rather than trusted. */
describe('the offline page', () => {
	const offline = rootTokens(OFFLINE);

	it('declares the console value for every token it names', () => {
		expect(offline.size).toBe(7);
		const drifted = [...offline]
			.filter(([name, value]) => expand(value) !== expand(colour(name)))
			.map(([name, value]) => `--${name}: offline ${value}, console ${colour(name)}`);
		expect(drifted).toEqual([]);
	});

	it("writes the card's shadow as --text at 8 per cent", () => {
		const hex = colour('text').replace('#', '');
		const channels = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16));
		expect(readFileSync(OFFLINE, 'utf8')).toContain(`rgba(${channels.join(', ')}, 0.08)`);
	});

	it('embeds the mark the console draws, unchanged', () => {
		const marks = [OFFLINE, FAVICON].map((path) => {
			const svg = readFileSync(path, 'utf8').replace(/<!--[\s\S]*?-->/g, '');
			const body = svg.slice(svg.indexOf('<svg'), svg.indexOf('</svg>'));
			return [...new Set([...body.matchAll(/#[0-9a-fA-F]{6}\b/g)].map(([hex]) => expand(hex)))].sort();
		});
		expect(marks[0].length).toBe(4);
		expect(marks[0]).toEqual(marks[1]);
	});
});
