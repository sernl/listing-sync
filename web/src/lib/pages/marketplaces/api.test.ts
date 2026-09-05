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

	it('keeps the entries that parse and drops the ones that do not', () => {
		const held = readManifest({
			version: '0.2.0',
			windows: { file: 'teachouse-0.2.0.msi', sha256: 'abc' },
			android: { file: 'teachouse-0.2.0.apk' },
			apple: null
		});
		expect(held).toEqual({
			version: '0.2.0',
			windows: { file: 'teachouse-0.2.0.msi', sha256: 'abc' },
			android: null,
			apple: null
		});
	});

	it('refuses a file name that would build a link outside the downloads path', () => {
		const escaping = ['../secrets.env', '/etc/passwd', 'https://elsewhere.test/x', '.hidden'];
		for (const file of escaping) {
			const held = readManifest({
				version: '0.2.0',
				windows: { file, sha256: 'abc' },
				android: null,
				apple: null
			});
			expect(held?.windows, file).toBeNull();
		}
	});
});

describe('the download link', () => {
	it('is built in one place, under the downloads path', () => {
		expect(downloadHref('teachouse-0.2.0.msi')).toBe('/downloads/teachouse-0.2.0.msi');
	});
});
