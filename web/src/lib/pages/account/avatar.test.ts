import { describe, expect, it } from 'vitest';
import { ApiFailure, avatarSrc } from '$lib/api';
import { NOT_SAVED, PICTURE_ONLY, avatarRefusal, pictureRefusal } from './avatar';

const HASH = '3fa405a8301ace34d11cf44a816080b8f0e49a48fbd048b8aef1543a8c58bdb6';

function refused(code: string, message: string, detail?: unknown): ApiFailure {
	return new ApiFailure(422, {
		status: 422,
		errors: [{ code, message, detail } as never]
	});
}

describe('the picture check before a byte is sent', () => {
	it('lets any image type through, because the server decides from the bytes', () => {
		expect(pictureRefusal('image/png')).toBeNull();
		expect(pictureRefusal('image/jpeg')).toBeNull();
		expect(pictureRefusal('image/webp')).toBeNull();
	});

	it('refuses a file the browser already knows is not a picture', () => {
		expect(pictureRefusal('application/pdf')).toBe(PICTURE_ONLY);
		expect(pictureRefusal('')).toBe(PICTURE_ONLY);
	});
});

describe('why a picture was not saved', () => {
	it('quotes the server where it named the slot', () => {
		const said = 'A profile picture has to be a picture: a JPEG, a PNG or a GIF.';
		expect(avatarRefusal(refused('upload_rejected', said))).toBe(said);
	});

	it('states the storage headroom for a quota refusal rather than the code', () => {
		const said = avatarRefusal(
			refused('quota_exceeded', 'quota exceeded', {
				quota: 'storage_bytes_max',
				used: 5_368_709_120,
				limit: 5_368_709_120
			})
		);
		expect(said).toMatch(/holds up to/);
	});

	it('says the deployment cannot store pictures where there is no store', () => {
		expect(avatarRefusal(refused('blob_store_unavailable', 'no store'))).toMatch(/not available yet/);
	});

	it('says the upload never arrived for a transport failure', () => {
		expect(avatarRefusal(new ApiFailure(0, null))).toMatch(/did not reach us/);
	});

	it('falls back to its own sentence where there is nothing to quote', () => {
		expect(avatarRefusal(new ApiFailure(502, null))).toBe(NOT_SAVED);
		expect(avatarRefusal(new TypeError('boom'))).toBe(NOT_SAVED);
	});
});

describe('where the picture is fetched from', () => {
	it('is the profile route, versioned by the hash so a changed picture is refetched', () => {
		expect(avatarSrc({ user: 'u', avatar_hash: HASH })).toBe(`/v1/profile/avatar?v=${HASH}`);
	});

	it('is nothing while the profile is unread or carries no picture', () => {
		expect(avatarSrc(undefined)).toBeNull();
		expect(avatarSrc({ user: 'u', avatar_hash: null })).toBeNull();
	});
});
