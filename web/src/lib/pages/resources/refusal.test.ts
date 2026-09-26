import { describe, expect, it } from 'vitest';
import { ApiFailure } from '$lib/api';
import { NOT_CREATED, createRefusal, sentenceFor } from './refusal';

describe('a refused create', () => {
	it('does not throw on a body that parses and carries no errors', () => {
		// The fault this guards: `{}` is truthy, so `body?.errors[0]` indexes
		// `undefined` and throws inside the create's own catch block, where
		// nothing catches it again. The seller saw no draft and no refusal.
		const bare = new ApiFailure(400, {} as unknown as { status: number; errors: [] });
		expect(() => createRefusal(bare)).not.toThrow();
		expect(createRefusal(bare)).toBe('request failed with 400');
	});

	it('reads the served sentence when the body carried one', () => {
		const refused = new ApiFailure(422, {
			status: 422,
			errors: [{ code: 'payload_missing', message: 'no payload', detail: undefined }]
		} as never);
		expect(createRefusal(refused)).toBe('Upload your file before creating the draft.');
	});

	it('does not hand the seller a status line when the body did not parse', () => {
		const proxied = new ApiFailure(502, null);
		expect(proxied.message).toBe('request failed with 502');
		expect(createRefusal(proxied)).toBe(NOT_CREATED);
	});

	it('says the draft was not created for anything that is not a refusal', () => {
		expect(createRefusal(new Error('offline'))).toBe(NOT_CREATED);
		expect(createRefusal(undefined)).toBe(NOT_CREATED);
	});
});

describe('a marketplace refusing a field this listing does not carry', () => {
	// The founder met this creating for Tes and read a sentence naming neither
	// the marketplace nor the field. The server always sent both.
	function refusedFor(detail: unknown) {
		return new ApiFailure(422, {
			status: 422,
			errors: [
				{
					code: 'required_field_missing',
					message: 'a selected platform requires a field this product does not carry',
					detail
				}
			]
		} as never);
	}

	it('names the marketplace and the field instead of the server sentence', () => {
		const said = createRefusal(refusedFor({ missing: [{ inventory: 'Tes', field: 'licence' }] }));
		expect(said).toContain('needs a licence');
		expect(said).toContain('does not have one yet');
		expect(said).not.toContain('a selected platform requires a field');
	});

	it('falls back to the server sentence for a detail it cannot read', () => {
		expect(createRefusal(refusedFor({ missing: 'licence' }))).toBe(
			'a selected platform requires a field this product does not carry'
		);
	});
});

describe('the sentence a page falls back to', () => {
	it('keeps the served message when a body carried one', () => {
		const refused = new ApiFailure(409, {
			status: 409,
			errors: [{ code: 'conflict', message: 'That listing is already attached.' }]
		} as never);
		expect(sentenceFor(refused, 'own')).toBe('That listing is already attached.');
	});

	it('uses the page own words when the body is absent', () => {
		expect(sentenceFor(new ApiFailure(500, null), 'That listing could not be attached.')).toBe(
			'That listing could not be attached.'
		);
	});
});
