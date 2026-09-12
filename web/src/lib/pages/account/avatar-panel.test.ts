// The picture panel and the shell's two tiles, read as source: the places a
// picture is chosen or drawn, pinned so a restyle cannot quietly drop the
// control, bypass the slot-bound upload, or let the top strip and the page
// header decide the tile differently.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const SETTINGS = readFileSync(
	new URL('../../../routes/settings/+page.svelte', import.meta.url),
	'utf8'
);
const CONSOLE = readFileSync(new URL('../../Console.svelte', import.meta.url), 'utf8');
const PAGE_HEAD = readFileSync(new URL('../../PageHead.svelte', import.meta.url), 'utf8');
const STORE = readFileSync(new URL('../../account-tile.svelte.ts', import.meta.url), 'utf8');

describe('the profile picture control', () => {
	it('is the resource form’s own drop panel, accepting pictures only', () => {
		expect(SETTINGS).toContain('<label class="drop" for="avatar-file">');
		expect(SETTINGS).toMatch(/id="avatar-file"[\s\S]*?type="file"[\s\S]*?accept="image\//);
	});

	it('uploads slot-bound and kept whole, then names that handle to the profile', () => {
		expect(SETTINGS).toContain("api.upload(file, 'keep_whole', undefined, 'image')");
		expect(SETTINGS).toContain('api.setAvatar(');
	});

	it('shows the picture it holds and offers to remove it', () => {
		expect(SETTINGS).toContain('class="acct-avatar-img"');
		expect(SETTINGS).toContain('api.clearAvatar()');
		expect(SETTINGS).toContain('Remove picture');
	});

	it('takes the server’s answer into the cache rather than the handle it sent', () => {
		expect(SETTINGS.match(/queryClient\.setQueryData\(queryKeys\.profile, stored\)/g)?.length).toBe(
			2
		);
	});
});

describe('the shell’s account tiles', () => {
	// Two components now: the top strip above 620px and the page header's phone
	// avatar below it. Both read the one store, so neither can decide the tile
	// for itself.
	it('are both decided by the shared store rather than by either component', () => {
		expect(STORE).toContain('accountTile(');
		for (const source of [CONSOLE, PAGE_HEAD]) {
			expect(source).toContain('accountTileState.tile');
			expect(source).not.toContain('accountTile(');
			expect(source).not.toContain('initialsOf');
		}
	});

	it('fall back to the initials when the picture will not draw', () => {
		for (const source of [CONSOLE, PAGE_HEAD]) {
			expect(source).toContain('accountTileState.unusable()');
		}
		expect(STORE).toContain('unshowable = picture');
	});
});
