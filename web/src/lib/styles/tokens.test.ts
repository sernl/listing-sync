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
	return new Map(
		[...body.matchAll(/--([a-z0-9-]+)\s*:\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]),
	);
}

const TOKENS = rootTokens(`${HERE}tokens.css`);

/** A token's colour, following `var()` aliases and resolving the one derived
 *  form this sheet uses: `color-mix(in srgb, var(--a) N%, var(--b))`.
 *
 *  The three verdict inks are written that way rather than as hexes, because
 *  the founder's kit contains no readable green, amber or red and a hand-typed
 *  darker one would be a colour the brand does not own. Resolving the mix here
 *  is what lets the contrast pairs below measure them as the browser paints
 *  them. sRGB, non-linear, which is what `color-mix(in srgb, ...)` specifies. */
function colour(name: string): string {
	const raw = TOKENS.get(name);
	if (raw === undefined) throw new Error(`tokens.css declares no --${name}`);
	const alias = raw.match(/^var\(--([a-z0-9-]+)\)$/);
	if (alias) return colour(alias[1]);
	const mix = raw.match(
		/^color-mix\(in srgb,\s*var\(--([a-z0-9-]+)\)\s+(\d+)%,\s*var\(--([a-z0-9-]+)\)\)$/,
	);
	if (!mix) return raw;
	const share = Number(mix[2]) / 100;
	const [a, b] = [colour(mix[1]), colour(mix[3])].map((hex) =>
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

/** A filled control's label against its own fill.
 *
 *  Teal is not here, because nothing fills a control with it and sets a word
 *  on top: white on the kit's Teal measures 2.54, so the accent fills discs
 *  and ticks, and `--primary` is what a solid button is. */
const ON_FILL: Array<[string, string]> = [
	['on-fill', 'primary'],
	['on-fill', 'additive'],
	['on-fill', 'text'],
	['surface', 'primary'],
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
		([literal]) => literal,
	);
}

describe('contrast', () => {
	/** A pair's own bar: `AA`, or `FLOOR` where the kit was granted a trade. */
	const barFor = (ink: string, ground: string) =>
		`${expand(colour(ink))} on ${expand(colour(ground))}` in KIT_TRADES ? FLOOR : AA;

	it.each(INKS)('--%s clears its bar on every ground a page is painted with', (ink) => {
		for (const ground of GROUNDS) {
			const bar = barFor(ink, ground);
			expect(`--${ink} on --${ground}: ${ratio(ink, ground).toFixed(2)}`).toBe(
				`--${ink} on --${ground}: ${Math.max(ratio(ink, ground), bar).toFixed(2)}`,
			);
		}
	});

	/** A trade the kit no longer needs is a trade that has to go, or the next
	 *  reader reads it as a standing licence. */
	it('every trade granted to the kit is still a trade', () => {
		const measured = new Map<string, number>();
		for (const ink of INKS) {
			for (const ground of [...GROUNDS, ...ON_SOFT.map(([, g]) => g)]) {
				measured.set(`${expand(colour(ink))} on ${expand(colour(ground))}`, ratio(ink, ground));
			}
		}
		const spent = Object.keys(KIT_TRADES).filter((pair) => (measured.get(pair) ?? AA) >= AA);
		expect(spent).toEqual([]);
	});

	it.each(ON_SOFT)('--%s clears its bar on --%s', (ink, ground) => {
		expect(ratio(ink, ground)).toBeGreaterThanOrEqual(barFor(ink, ground));
	});

	it.each(ON_FILL)('--%s clears AA on --%s', (ink, ground) => {
		expect(ratio(ink, ground)).toBeGreaterThanOrEqual(AA);
	});

	it('every label hue clears AA on the chip and on the card', () => {
		expect(LABELS.length).toBe(7);
		const failing = LABELS.flatMap((hue) =>
			(['ground', 'surface'] as const)
				.filter((ground) => ratio(hue, ground) < AA)
				.map((ground) => `--${hue} on --${ground}: ${ratio(hue, ground).toFixed(2)}`),
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
	const landing = rootTokens(LANDING_CSS);
	const shared = [...landing.keys()].filter((name) => TOKENS.has(name));

	/** The landing's own value for a token, aliases followed in its own sheet.
	 *  Both sheets write `--card: var(--surface)`, so the comparison has to
	 *  resolve each side against the sheet it is written in or every alias
	 *  reads as drift. */
	const landingColour = (name: string): string => {
		const raw = landing.get(name) ?? '';
		const alias = raw.match(/^var\(--([a-z0-9-]+)\)$/);
		return alias ? landingColour(alias[1]) : raw;
	};

	/** No token is allowed to differ. The founder's kit of 2026-09-11 is one
	 *  brand across the marketing site and the console, which retired the one
	 *  difference this file used to grant: the landing's 2rem section card
	 *  against the console's 16px, now `--r-card` in both. */
	it('the landing and the console agree on every token both declare', () => {
		expect(shared.length).toBeGreaterThan(10);
		const drifted = shared
			.filter((name) => expand(landingColour(name)) !== expand(colour(name)))
			.map((name) => `--${name}: landing ${landing.get(name)}, console ${colour(name)}`);
		expect(drifted).toEqual([]);
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

	const declared = new Set([...TOKENS.keys()].map((name) => expand(colour(name))));

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
	const offline = rootTokens(OFFLINE);

	it('declares the console value for every token it names', () => {
		expect(offline.size).toBe(6);
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
