import { describe, expect, it } from 'vitest';
import { NAME_MAX_CHARS, charCount, downloadHref, isWebAddress, readManifest } from './api';

describe('counting a field the way the server counts it', () => {
	it('counts a plain string as its length', () => {
		expect(charCount('Made By Teachers')).toBe(16);
	});

	it('counts an astral character once, as a Rust char is counted', () => {
		// Two UTF-16 code units, one scalar value. Counting the JavaScript way
		// would refuse a name the server would have taken.
		expect('🎓'.length).toBe(2);
		expect(charCount('🎓')).toBe(1);
	});

	it('agrees with the server at the bound', () => {
		expect(charCount('🎓'.repeat(NAME_MAX_CHARS))).toBe(NAME_MAX_CHARS);
	});
});

describe('the web address test', () => {
	it('takes an http or https address naming a host', () => {
		expect(isWebAddress('https://classful.com/')).toBe(true);
		expect(isWebAddress('http://teachbuysell.com.au')).toBe(true);
	});

	it('takes a scheme typed in capitals, which is still an address', () => {
		expect(isWebAddress('HTTPS://eduki.com')).toBe(true);
	});

	it('refuses a scheme we cannot open and one with no host', () => {
		expect(isWebAddress('ftp://example.com')).toBe(false);
		expect(isWebAddress('example.com')).toBe(false);
		expect(isWebAddress('https://')).toBe(false);
		expect(isWebAddress('https:// spaced.com')).toBe(false);
	});

	it('keeps a path, a query and a port, which are all parts of a real shop address', () => {
		expect(isWebAddress('https://example.com:8443/shop?ref=1')).toBe(true);
	});
});

describe('reading the download manifest', () => {
	it('answers null for anything that is not a manifest', () => {
		expect(readManifest(null)).toBeNull();
		expect(readManifest('nope')).toBeNull();
		expect(readManifest({})).toBeNull();
		expect(readManifest({ version: '' })).toBeNull();
	});

	it('keeps the entries and files that parse and drops the ones that do not', () => {
		const exe = { kind: 'exe', file: 'Teachouse_0.2.0_x64-setup.exe', sha256: 'abc', size: 10 };
		const held = readManifest({
			version: '0.2.0',
			windows: {
				version: '0.2.0',
				updated: '2026-10-05T00:00:00Z',
				files: [exe, { kind: 'zip', file: 'Teachouse.zip', sha256: 'x' }],
				store: null
			},
			macos: { version: '0.2.0', files: [{ kind: 'dmg', file: 'Teachouse.dmg' }] },
			// Files with no version are not offered: the card would print none.
			linux: { files: [{ kind: 'appimage', file: 'Teachouse.AppImage', sha256: 'def' }] },
			android: null
		});
		expect(held).toEqual({
			version: '0.2.0',
			windows: { version: '0.2.0', updated: '2026-10-05T00:00:00Z', files: [exe], store: null },
			macos: null,
			linux: null,
			android: null
		});
	});

	it('refuses a file name that would build a link outside the downloads path', () => {
		const escaping = ['../secrets.env', '/etc/passwd', 'https://elsewhere.test/x', '.hidden'];
		for (const file of escaping) {
			const held = readManifest({
				version: '0.2.0',
				windows: { version: '0.2.0', files: [{ kind: 'exe', file, sha256: 'abc' }] }
			});
			expect(held?.windows, file).toBeNull();
		}
	});

	it('keeps a store link only where it is that platform’s own store, over https', () => {
		const listing = (platform: string, store: string) =>
			readManifest({ version: '0.2.0', [platform]: { files: [], store } });
		expect(
			listing('android', 'https://play.google.com/store/apps/details?id=io.teachouse.desktop')
				?.android?.store
		).toBe('https://play.google.com/store/apps/details?id=io.teachouse.desktop');
		expect(listing('android', 'http://play.google.com/x')?.android).toBeNull();
		expect(listing('android', 'https://apps.microsoft.com/detail/9n')?.android).toBeNull();
		expect(listing('windows', 'https://apps.microsoft.com/detail/9n')?.windows?.store).toBe(
			'https://apps.microsoft.com/detail/9n'
		);
		expect(listing('macos', 'https://apps.apple.com/app/id1')?.macos?.store).toBe(
			'https://apps.apple.com/app/id1'
		);
		expect(listing('linux', 'https://flathub.org/apps/x')?.linux).toBeNull();
	});
});

describe('the download link', () => {
	it('is built in one place, under the downloads path', () => {
		expect(downloadHref('teachouse-0.2.0.msi')).toBe('/downloads/teachouse-0.2.0.msi');
	});
});
