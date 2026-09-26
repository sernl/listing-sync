/**
 * What the preview maker was asked for, kept beside the preview it made so
 * the seller can reopen the maker on the same choices and change them.
 *
 * Kept in this browser's storage, keyed by the made preview's digest: the
 * digest is the preview's name in both modes (a draft's handle and a saved
 * resource's file carry the same one), so a recipe written before Create is
 * still found after it. Nothing here reaches the server; a preview made on
 * another machine simply has no recipe and the maker opens on its defaults.
 */

export interface PreviewRecipe {
	/** Source pages, 1-based, in the order the preview carries them. */
	pages: number[];
	/** Source pages the watermark is drawn on. */
	marked: number[];
	watermark: boolean;
	watermarkText: string;
}

const PREFIX = 'teachouse.preview-recipe.';

export function recipeKey(hash: string): string {
	return `${PREFIX}${hash}`;
}

function pageList(value: unknown): number[] | null {
	if (!Array.isArray(value)) {
		return null;
	}
	const pages = value.filter(
		(page): page is number => typeof page === 'number' && Number.isInteger(page) && page >= 1
	);
	return pages.length === value.length ? pages : null;
}

/** A stored recipe, or `null` for anything that is not one: storage is
 *  shared with every other script on this origin and an older build. */
export function parseRecipe(raw: string | null): PreviewRecipe | null {
	if (raw === null) {
		return null;
	}
	let value: unknown;
	try {
		value = JSON.parse(raw);
	} catch {
		return null;
	}
	if (typeof value !== 'object' || value === null) {
		return null;
	}
	const record = value as Record<string, unknown>;
	const pages = pageList(record.pages);
	const marked = pageList(record.marked);
	if (
		pages === null ||
		pages.length === 0 ||
		marked === null ||
		typeof record.watermark !== 'boolean' ||
		typeof record.watermarkText !== 'string'
	) {
		return null;
	}
	return { pages, marked, watermark: record.watermark, watermarkText: record.watermarkText };
}

function storage(): Storage | null {
	try {
		return typeof localStorage === 'undefined' ? null : localStorage;
	} catch {
		return null;
	}
}

export function readRecipe(hash: string): PreviewRecipe | null {
	return parseRecipe(storage()?.getItem(recipeKey(hash)) ?? null);
}

/** Best effort: a full or disabled storage loses the prefill, not the preview. */
export function writeRecipe(hash: string, recipe: PreviewRecipe): void {
	try {
		storage()?.setItem(recipeKey(hash), JSON.stringify(recipe));
	} catch {
		// Nothing to tell the seller: the maker opens on its defaults instead.
	}
}

export function forgetRecipe(hash: string): void {
	try {
		storage()?.removeItem(recipeKey(hash));
	} catch {
		// As above.
	}
}
