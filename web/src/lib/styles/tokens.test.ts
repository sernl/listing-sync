// The palette's own gate: contrast on both grounds, drift between the sheets
// that declare it, and the absence of any colour this file does not name.
//
// It exists because the palette is the one thing in this console that is
// correct by arithmetic rather than by looking right, and arithmetic is exactly
// what a later edit made for looks will quietly break. Every value below is
// read from `tokens.css` rather than restated here, so nudging a token one
// shade fails the lane instead of shipping.
//
// Since 2026-09-12 the sheet declares two palettes, and every pair below is
// measured on both. Dark is not granted the trades light was: the kit's Slate
// on the kit's navigation card is a documented 4.29 there, and on the dark
// ground the ink names split precisely so that nothing has to be excused.

import { readFileSync, readdirSync, existsSync, statSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const HERE = new URL('.', import.meta.url).pathname;
const WEB_SRC = new URL('../../', import.meta.url).pathname;
const REPO = new URL('../../../../', import.meta.url).pathname;
const LANDING_CSS = `${REPO}apps/landing/src/styles/site.css`;
const STATIC = `${REPO}web/static`;
const OFFLINE = `${STATIC}/unreachable.html`;
const FAVICON = `${STATIC}/favicon.svg`;

/** The declaring blocks of a stylesheet: `:root` and the dark override, in
 *  that order and with comments dropped.
 *
 *  Matched on the selector rather than on position, because the console's
 *  sheet ends with a third block (`[data-theme='light']`) that pins
 *  `color-scheme` and declares no token.
 *
 *  Two spellings of dark, because the two kinds of file reach it differently.
 *  A page the console serves is painted from the seller's stored choice, so
 *  the attribute is what selects; `unreachable.html` is opened from the
 *  desktop bundle with no script and no storage, so the machine's own
 *  preference is the only signal it has and it uses the media query. Both
 *  are the same table and both are checked against it. */
const THEME_SELECTORS: Record<Theme, RegExp> = {
	light: /:root\s*\{/,
	dark: /(?:\[data-theme=['"]?dark['"]?\]|@media \(prefers-color-scheme: dark\)\s*\{\s*:root)\s*\{/,
};

type Theme = 'light' | 'dark';

const THEMES: readonly Theme[] = ['light', 'dark'];

/** The body of one declaring block, comments dropped. */
function block(path: string, theme: Theme): string {
	const sheet = readFileSync(path, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
	const opens = THEME_SELECTORS[theme].exec(sheet);
	if (!opens) throw new Error(`${path} declares no ${theme} block`);
	const body = sheet.slice(opens.index + opens[0].length);
	return body.slice(0, body.indexOf('}'));
}

/** Every `--name: value` inside one declaring block. */
function blockTokens(path: string, theme: Theme): Map<string, string> {
	return new Map(
		[...block(path, theme).matchAll(/--([a-z0-9-]+)\s*:\s*([^;]+);/g)].map((m) => [
			m[1],
			m[2].trim(),
		]),
	);
}

/** What a sheet actually resolves to under one theme: the light table with
 *  the dark block's overrides applied, which is what the cascade does and
 *  therefore what a seller sees. A dark block that declares six tokens is a
 *  palette of every light token but six. */
function table(path: string, theme: Theme): Map<string, string> {
	const light = blockTokens(path, 'light');
	if (theme === 'light') return light;
	return new Map([...light, ...blockTokens(path, 'dark')]);
}

const SHEET = `${HERE}tokens.css`;
const TABLES: Record<Theme, Map<string, string>> = {
	light: table(SHEET, 'light'),
	dark: table(SHEET, 'dark'),
};
const TOKENS = TABLES.light;

/** A token's colour, following `var()` aliases and resolving the one derived
 *  form this sheet uses: `color-mix(in srgb, var(--a) N%, var(--b))`.
 *
 *  The three verdict inks are written that way rather than as hexes, because
 *  the founder's kit contains no readable green, amber or red and a hand-typed
 *  darker one would be a colour the brand does not own. Resolving the mix here
 *  is what lets the contrast pairs below measure them as the browser paints
 *  them. sRGB, non-linear, which is what `color-mix(in srgb, ...)` specifies. */
function colour(name: string, theme: Theme = 'light'): string {
	const raw = TABLES[theme].get(name);
	if (raw === undefined) throw new Error(`tokens.css declares no --${name}`);
	const alias = raw.match(/^var\(--([a-z0-9-]+)\)$/);
	if (alias) return colour(alias[1], theme);
	const mix = raw.match(
		/^color-mix\(in srgb,\s*var\(--([a-z0-9-]+)\)\s+(\d+)%,\s*var\(--([a-z0-9-]+)\)\)$/,
	);
	if (!mix) return raw;
	const share = Number(mix[2]) / 100;
	const [a, b] = [colour(mix[1], theme), colour(mix[3], theme)].map((hex) =>
		[1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)),
	);
	const blend = a.map((v, i) => Math.round(v * share + b[i] * (1 - share)));
	return `#${blend.map((v) => v.toString(16).padStart(2, '0')).join('')}`;
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

/** WCAG 2.x contrast between two token names, on one of the two grounds. */
function ratio(ink: string, ground: string, theme: Theme = 'light'): number {
	const a = luminance(colour(ink, theme));
	const b = luminance(colour(ground, theme));
	return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

/** The bar for every pair here.
 *
 *  4.5 rather than the 3.0 large-text allowance, with no exception for the
 *  status badges: a badge in this console is 10.5px uppercase, which is smaller
 *  than body text rather than larger, so nothing on screen earns the
 *  allowance. */
const AA = 4.5;

/** The floor under a pair the founder's kit does not carry at `AA`, listed in
 *  `KIT_TRADES` below. 4.0 is not a second bar anything is designed to: it is
 *  what keeps a granted trade from drifting further, so a kit colour that
 *  moves another half point fails the lane instead of being covered by the
 *  exception it was granted at a different value. */
const FLOOR = 4.0;

/** Every neutral a page is painted with, which is what body text sits on.
 *
 *  A saturated fill such as `--accent` is a ground too, but only under the ink
 *  the pairs below name for it, so it is checked there rather than swept in
 *  here. */
const GROUNDS = ['ground', 'surface', 'nav', 'hover', 'tint', 'tint-edge'];

/** Every token drawn as text somewhere in the console.
 *
 *  The kit's saturated colours are not on it, and that is the brand decision
 *  of 2026-09-11 rather than an omission: Teal measures 2.42 against the page,
 *  Success 2.42 and Warning 2.05, so each of them is a fill, a dot or a track
 *  and a sentence that has to carry their meaning takes `--primary` or one of
 *  the three derived `-ink` tokens. `every token drawn as text` below asserts
 *  this list is still complete, so a rule that starts drawing a kit fill as
 *  text fails the lane rather than shipping a word nobody can read. */
const INKS = [
	'text',
	'ink',
	'muted',
	'faint',
	'primary',
	'additive',
	'soon',
	'ok-ink',
	'warn-ink',
	'bad-ink',
];

/** The pairs the founder's kit does not carry at `AA`, each with the reason it
 *  is a decision rather than drift, and each keyed by the two colours rather
 *  than by the two names: the kit has one Slate, and `--muted`, `--faint` and
 *  `--soon` are three names for it, so one entry answers for all three.
 *
 *  The assertions below refuse an entry whose pair now clears `AA`, so a trade
 *  cannot outlive the kit colour it was granted for. */
const KIT_TRADES: Record<string, string> = {
	'#64748b on #f1f3f9':
		"The kit's Slate on the kit's navigation ground: 4.29. The card's own items are `--text`; what Slate draws there is the item's icon and the chip beside it.",
	'#64748b on #eef1f7':
		"The kit's Slate on the kit's hover ground, and on `--soon-soft`, which is the same colour: 4.21. A hover is transient and the resting ground clears the bar at 4.54.",
	'#64748b on #efe9ff':
		"The kit's Slate on the violet wash: 4.03. The wash is the marketing hero's, and the console draws no secondary text on it.",
	'#64748b on #f5f2ff':
		"The kit's Slate on the wash's edge: 4.31, and the same reason as the wash itself.",
};

/** A badge's ink against the soft ground that badge is painted on. */
const ON_SOFT: Array<[string, string]> = [
	['primary', 'accent-soft'],
	['additive', 'additive-soft'],
	['ok-ink', 'ok-soft'],
	['warn-ink', 'warn-soft'],
	['bad-ink', 'bad-soft'],
	['soon', 'soon-soft'],
];

/** A filled control's label against its own fill, and the rail's icons
 *  against the rail.
 *
 *  Teal is not here, because nothing fills a control with it and sets a word
 *  on top: white on the kit's Teal measures 2.54, so the accent fills discs
 *  and ticks, and `--primary` is what a solid button is.
 *
 *  The rail is here rather than in `GROUNDS` because it is not a ground a
 *  page is painted with: it is one band, `--rail-ink` is the only thing drawn
 *  on it, and on dark it is the one surface that stays Indigo while
 *  `--primary` moves to Lavender. */
const ON_FILL: Array<[string, string]> = [
	['on-fill', 'primary'],
	['on-fill', 'additive'],
	['on-fill', 'text'],
	['surface', 'primary'],
	['rail-ink', 'rail-fill'],
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

/** Paths under `web/static` this palette does not reach, each with the reason
 *  it is a decision rather than drift. A key is matched as a prefix, so it
 *  names either a directory or one file. */
const STATIC_DIRS: Record<string, string> = {
	'marketplaces/':
		"Each marketplace's own logo, drawn in colours that are the marketplace's and not ours to move.",
	'vendors/':
		"Mozilla's Firefox logo and Google's Android robot, each the vendor's published file unaltered. Both licences forbid modifying the mark, so its colours are the vendor's and not ours to move.",
	'email/teachouse-delivery.svg':
		"An illustration rather than a mark: a figure at a door, with skin tones, a satchel and a sky that no token names and that a palette edit has no opinion about. Its two brand colours were moved to the kit's Indigo and Teal on 2026-09-11 with the geometry untouched; the rest is the drawing's own. The two marks beside it are the console's mark and are swept.",
};

/** Everything else `web/static` writes that is neither a token's value nor the
 *  mark's own. */
const STATIC_OTHER: Record<string, string> = {
	'rgba(':
		"The offline card's shadow, which is `--sh-2` written out: `color-mix` is what the token uses and the webview the desktop app bundles may predate it. The channels are checked against `--text` below.",
};

/** Every colour literal a file writes, comments dropped.
 *
 *  `white` and `black` are here beside the hexes since 2026-09-12, and they
 *  are the reason the dark block found four unreadable bands on the marketing
 *  site rather than shipping them: `color-mix(in srgb, var(--peach) 35%,
 *  white)` reads as a token to a sweep that looks for `#` and a paren, and it
 *  is a hard-coded Cream to a browser. A keyword ground does not follow a
 *  theme, so it is a literal exactly as `#ffffff` is. The `(?!-)` keeps
 *  `white-space` out of it. */
function literalsIn(path: string): string[] {
	// Comments go first: a hex inside one is prose about the palette, not a
	// colour the browser will paint, and this file's own header says
	// `color: #fff` while explaining why none may remain. The third form is
	// the line comment a Svelte component's script block writes; `(?<!:)`
	// keeps a `https://` out of it, and CSS has no such comment so a sheet is
	// unaffected.
	const body = readFileSync(path, 'utf8')
		.replace(/\/\*[\s\S]*?\*\//g, '')
		.replace(/<!--[\s\S]*?-->/g, '')
		.replace(/(?<!:)\/\/.*$/gm, '');
	const sheet =
		path.endsWith('tokens.css') || path === LANDING_CSS
			? // Both declaring blocks, because a palette is where a hex is
				// allowed to be written and there are two of them now. The
				// `[data-theme='light']` block declares no token and is left
				// in the sweep, which is the point: a colour written there
				// would be a third palette nobody is reading.
				body.replace(/(?::root|\[data-theme=['"]?dark['"]?\])\s*\{[\s\S]*?\n\}/g, '')
			: body;
	return [
		...sheet.matchAll(
			/#[0-9a-fA-F]{3}\b|#[0-9a-fA-F]{6}\b|\brgba?\(|\bhsla?\(|\bwhite\b(?!-)|\bblack\b(?!-)/g,
		),
	].map(([literal]) => literal);
}

describe('contrast', () => {
	/** A pair's own bar: `AA`, or `FLOOR` where the kit was granted a trade.
	 *
	 *  Keyed by the two colours rather than the two names, so a trade granted
	 *  on light cannot silently cover a different pair of hexes on dark. */
	const barFor = (ink: string, ground: string, theme: Theme) =>
		`${expand(colour(ink, theme))} on ${expand(colour(ground, theme))}` in KIT_TRADES
			? FLOOR
			: AA;

	const pairs = THEMES.flatMap((theme) => INKS.map((ink): [Theme, string] => [theme, ink]));

	it.each(pairs)('on %s, --%s clears its bar on every ground a page is painted with', (theme, ink) => {
		for (const ground of GROUNDS) {
			const bar = barFor(ink, ground, theme);
			const measured = ratio(ink, ground, theme);
			expect(`${theme}: --${ink} on --${ground}: ${measured.toFixed(2)}`).toBe(
				`${theme}: --${ink} on --${ground}: ${Math.max(measured, bar).toFixed(2)}`,
			);
		}
	});

	/** A trade the kit no longer needs is a trade that has to go, or the next
	 *  reader reads it as a standing licence. Measured across both palettes,
	 *  so a trade whose light pair went away cannot survive on a dark pair it
	 *  was never granted for. */
	it('every trade granted to the kit is still a trade', () => {
		const measured = new Map<string, number>();
		for (const theme of THEMES) {
			for (const ink of INKS) {
				for (const ground of [...GROUNDS, ...ON_SOFT.map(([, g]) => g)]) {
					const pair = `${expand(colour(ink, theme))} on ${expand(colour(ground, theme))}`;
					measured.set(pair, ratio(ink, ground, theme));
				}
			}
		}
		const spent = Object.keys(KIT_TRADES).filter((pair) => (measured.get(pair) ?? AA) >= AA);
		expect(spent).toEqual([]);
	});

	/** Dark earns no exception, and the emptiness of this list is the claim:
	 *  every trade in `KIT_TRADES` is a light pair. */
	it('the dark palette is granted no trade', () => {
		const granted = [];
		for (const ink of INKS) {
			for (const ground of [...GROUNDS, ...ON_SOFT.map(([, g]) => g)]) {
				const pair = `${expand(colour(ink, 'dark'))} on ${expand(colour(ground, 'dark'))}`;
				if (pair in KIT_TRADES) granted.push(pair);
			}
		}
		expect(granted).toEqual([]);
	});

	it.each(THEMES.flatMap((theme) => ON_SOFT.map(([i, g]) => [theme, i, g])))(
		'on %s, --%s clears its bar on --%s',
		(theme, ink, ground) => {
			const on = theme as Theme;
			expect(ratio(ink, ground, on)).toBeGreaterThanOrEqual(barFor(ink, ground, on));
		},
	);

	it.each(THEMES.flatMap((theme) => ON_FILL.map(([i, g]) => [theme, i, g])))(
		'on %s, --%s clears AA on --%s',
		(theme, ink, ground) => {
			expect(ratio(ink, ground, theme as Theme)).toBeGreaterThanOrEqual(AA);
		},
	);

	it.each(THEMES)('on %s, every label hue clears AA on the chip and on the card', (theme) => {
		expect(LABELS.length).toBe(7);
		const failing = LABELS.flatMap((hue) =>
			(['ground', 'surface'] as const)
				.filter((ground) => ratio(hue, ground, theme) < AA)
				.map((ground) => `--${hue} on --${ground}: ${ratio(hue, ground, theme).toFixed(2)}`),
		);
		expect(failing).toEqual([]);
	});

	it.each(THEMES)('on %s, the label hues stay seven distinct colours', (theme) => {
		expect(new Set(LABELS.map((hue) => colour(hue, theme))).size).toBe(LABELS.length);
	});

	it('every token drawn as text is one the pairs above measure', () => {
		const drawn = new Set<string>();
		for (const file of filesUnder(WEB_SRC, ['.css', '.svelte'])) {
			for (const [, name] of readFileSync(file, 'utf8').matchAll(
				/(?<!-)\bcolor:\s*var\(--([a-z0-9-]+)\)/g,
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

describe('one palette, declared once', () => {
	const landingTable: Record<Theme, Map<string, string>> = {
		light: table(LANDING_CSS, 'light'),
		dark: table(LANDING_CSS, 'dark'),
	};
	const shared = [...landingTable.light.keys()].filter((name) => TOKENS.has(name));

	/** The landing's own value for a token, aliases followed in its own sheet.
	 *  Both sheets write `--card: var(--surface)`, so the comparison has to
	 *  resolve each side against the sheet it is written in or every alias
	 *  reads as drift. */
	const landingColour = (name: string, theme: Theme): string => {
		const raw = landingTable[theme].get(name) ?? '';
		const alias = raw.match(/^var\(--([a-z0-9-]+)\)$/);
		return alias ? landingColour(alias[1], theme) : raw;
	};

	/** No token is allowed to differ. The founder's kit of 2026-09-11 is one
	 *  brand across the marketing site and the console, which retired the one
	 *  difference this file used to grant: the landing's 2rem section card
	 *  against the console's 16px, now `--r-card` in both.
	 *
	 *  Both palettes, because one brand in two themes is still one brand: a
	 *  dark block on the console that the site did not follow would put a
	 *  seller's own marketing page in a different indigo from their console. */
	it.each(THEMES)('on %s, the landing and the console agree on every token both declare', (theme) => {
		expect(shared.length).toBeGreaterThan(10);
		const drifted = shared
			.filter((name) => expand(landingColour(name, theme)) !== expand(colour(name, theme)))
			.map(
				(name) =>
					`--${name}: landing ${landingColour(name, theme)}, console ${colour(name, theme)}`,
			);
		expect(drifted).toEqual([]);
	});

	/** A shared token the console re-grounds and the site does not is the
	 *  drift the test above cannot see on its own: the site would inherit its
	 *  light value into the dark block and read as a lighter brand rather than
	 *  as a different one. */
	it('the two sheets re-ground the same shared names on dark', () => {
		const overridden = (path: string) =>
			[...blockTokens(path, 'dark').keys()].filter((name) => shared.includes(name)).sort();
		expect(overridden(LANDING_CSS)).toEqual(overridden(SHEET));
	});

	/** Both schemes on the root, and each block pinning its own, so a browser
	 *  paints its form controls, scrollbars and canvas to match the ground the
	 *  page is actually drawn on rather than the one the reader's system
	 *  prefers. A sheet that stops declaring this hands all three back. */
	it('both sheets declare both schemes and pin each one', () => {
		const wrong = [SHEET, LANDING_CSS]
			.filter(
				(path) =>
					!/\bcolor-scheme:\s*light dark\s*;/.test(block(path, 'light')) ||
					!/\bcolor-scheme:\s*dark\s*;/.test(block(path, 'dark')) ||
					!/\[data-theme=['"]?light['"]?\]\s*\{\s*color-scheme:\s*light\s*;/.test(
						readFileSync(path, 'utf8').replace(/\/\*[\s\S]*?\*\//g, ''),
					),
			)
			.map((path) => path.slice(REPO.length));
		expect(wrong).toEqual([]);
	});

	/** The sheets and templates that must reach every colour through a token,
	 *  and `web/static`, which cannot: the desktop app opens `unreachable.html`
	 *  from its own bundle with no stylesheet and no network, and an SVG has no
	 *  tokens to read, so both spell their colours out. A literal there is
	 *  allowed only where it is a token's own value -- a copy the palette still
	 *  owns, and one that stops matching the moment a token moves -- or where it
	 *  is named above. The mark has no colours of its own under the brand kit:
	 *  it is drawn in Indigo and Teal and nothing else, which is what retired
	 *  the two creams the Pounamu and Kauri marks needed an exception for. */
	const swept = () => [
		...filesUnder(WEB_SRC, ['.css', '.svelte']),
		...filesUnder(`${REPO}apps/landing/src`, ['.css', '.svelte', '.astro']),
		...filesUnder(STATIC, ['.html', '.svg']).filter(
			(file) => !Object.keys(STATIC_DIRS).some((dir) => file.startsWith(`${STATIC}/${dir}`)),
		),
	];

	/** Both palettes: `unreachable.html` carries a copy of each, so a dark
	 *  ground written there is a token's own value even though no light token
	 *  names it. */
	const declared = new Set(
		THEMES.flatMap((theme) => [...TABLES[theme].keys()].map((name) => expand(colour(name, theme)))),
	);

	it('every colour on screen is a token, by var() or by a checked copy', () => {
		const stray: string[] = [];
		for (const file of swept()) {
			for (const literal of literalsIn(file)) {
				const written = expand(literal);
				const allowed =
					file.startsWith(`${STATIC}/`) && (declared.has(written) || written in STATIC_OTHER);
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
				.flatMap((file) => literalsIn(file).map(expand)),
		);
		const all = filesUnder(STATIC, ['.html', '.svg']);
		const spent = [
			...Object.keys(STATIC_DIRS).filter(
				(dir) =>
					!all.some((file) => file.startsWith(`${STATIC}/${dir}`) && literalsIn(file).length > 0),
			),
			// A file the exception names that no longer writes a colour has
			// nothing left to except, and the exception goes with it.
			...Object.keys(STATIC_OTHER).filter((literal) => !written.has(literal)),
		];
		expect(spent).toEqual([]);
	});
});

/** The console's palette reaches the desktop app's offline page as a copy,
 *  because that page is opened from the app's own bundle when the server
 *  cannot be reached and so has no stylesheet to import. The copy is checked
 *  rather than trusted. */
describe('the offline page', () => {
	it.each(THEMES)('declares the console %s value for every token it names', (theme) => {
		const offline = blockTokens(OFFLINE, theme);
		expect(offline.size).toBe(6);
		const drifted = [...offline]
			.filter(([name, value]) => expand(value) !== expand(colour(name, theme)))
			.map(([name, value]) => `--${name}: offline ${value}, console ${colour(name, theme)}`);
		expect(drifted).toEqual([]);
	});

	/** The page has no script and no stylesheet, so it cannot read the
	 *  seller's stored choice; it follows the machine instead, which is the
	 *  only signal a bundled file with no JavaScript has. */
	it('reaches its dark block through the media query rather than the attribute', () => {
		expect(readFileSync(OFFLINE, 'utf8')).toContain('@media (prefers-color-scheme: dark)');
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
			return [
				...new Set([...body.matchAll(/#[0-9a-fA-F]{6}\b/g)].map(([hex]) => expand(hex))),
			].sort();
		});
		expect(marks[0].length).toBe(2);
		expect(marks[0]).toEqual(marks[1]);
	});
});

/** No box in the client is grounded on the ink.
 *
 *  The ink names a colour dark enough to be read as black, and a card painted
 *  with it is the one surface in this console that stopped looking like the
 *  console. The tokens are derived rather than listed, because the ink already
 *  answers to two names -- `--text` and the `--ink` alias -- and a third would
 *  otherwise walk past a hand-written pair. The comparison goes through
 *  `expand` for the same reason every other value comparison in this file
 *  does: the same ink written `#17231C` is the same ink. Only a direct
 *  resolution counts: a `color-mix` holding the ink is a tint or a scrim, and
 *  several are drawn deliberately. The pin below is the emptiness guard rather
 *  than a second hand-written list -- a derivation that returned nothing would
 *  leave the sweep with no token to look for and pass vacuously -- so a
 *  palette event that adds or renames an ink fails there to be acknowledged
 *  rather than silently widening the sweep. */
describe('no surface is painted with the ink', () => {
	const inks = [...TOKENS.keys()].filter((name) => expand(colour(name)) === expand(colour('text')));

	it('names the ink and its alias', () => {
		expect([...inks].sort()).toEqual(['ink', 'text']);
	});

	it('no rule under web/src grounds a box on one of them', () => {
		const painted: string[] = [];
		for (const file of filesUnder(WEB_SRC, ['.css', '.svelte'])) {
			const body = readFileSync(file, 'utf8')
				.replace(/\/\*[\s\S]*?\*\//g, '')
				.replace(/<!--[\s\S]*?-->/g, '');
			for (const [rule] of body.matchAll(
				new RegExp(`background(?:-color)?:\\s*var\\(--(?:${inks.join('|')})\\)`, 'g'),
			)) {
				painted.push(`${file.slice(REPO.length)}: ${rule}`);
			}
		}
		expect(painted).toEqual([]);
	});
});
