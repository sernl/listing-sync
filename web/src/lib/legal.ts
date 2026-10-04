// The agreement every account makes before it exists, and makes again when the
// terms change: the two boxes' words, where the documents live, and the
// form state that decides whether the agreement can be sent. Pure, so it tests
// without a component.

import { PRIVACY_VERSION, TERMS_VERSION } from '$lib/generated/legal';
import { MARKETING_URL } from '$lib/site';

export { PRIVACY_VERSION, TERMS_VERSION };

/** The documents themselves, on the marketing site. */
export const TERMS_URL = `${MARKETING_URL}/terms/`;
export const PRIVACY_URL = `${MARKETING_URL}/privacy/`;

/** Box (a), in the pieces the links sit between: "I agree to the" [Terms of
 *  Service] "and the" [Privacy Policy] ", and I confirm…". */
export const TERMS_BOX = {
	lead: 'I agree to the',
	terms: 'Terms of Service',
	join: 'and the',
	privacy: 'Privacy Policy',
	tail: ', and I confirm that I own or hold the rights to every resource I publish through Teachouse.'
} as const;

/** Box (b). */
export const AGE_BOX = 'I am 18 or older.';

/** What the identity service and the API both say when a box is unticked. */
export const CONSENT_REFUSAL = 'Please tick both boxes to continue.';

/** The two boxes as the form holds them. */
export interface ConsentBoxes {
	/** Box (a): the terms, the privacy policy and owning what they publish. */
	terms: boolean;
	/** Box (b): 18 or older. */
	age: boolean;
}

/** The agreement as it goes over the wire, to the identity service at sign-up
 *  and to `POST /v1/consent` afterwards. Box (a) is two facts recorded
 *  separately, so it sets two flags. */
export interface ConsentBody {
	terms_privacy: boolean;
	ip_ownership: boolean;
	age_18: boolean;
	version: string;
}

/** Whether both boxes are ticked, which is the only state that may be sent. */
export function consentReady(boxes: ConsentBoxes): boolean {
	return boxes.terms && boxes.age;
}

/** The agreement to send, or null until both boxes are ticked. */
export function consentBody(
	boxes: ConsentBoxes,
	version: string = TERMS_VERSION
): ConsentBody | null {
	if (!consentReady(boxes)) {
		return null;
	}
	return { terms_privacy: true, ip_ownership: true, age_18: true, version };
}

/** The sentences for the `?error=` codes a social sign-in or sign-up returns
 *  with, keyed by the code; null for a code that is not about the agreement. */
export function socialConsentError(code: string | null): string | null {
	switch (code) {
		case 'signup_disabled':
			return 'No Teachouse account uses that sign-in yet. Create one on the sign-up page first.';
		case 'consent_unavailable':
			return 'We could not record your agreement, so your account was not created. Please try again in a minute.';
		case 'unable_to_create_user':
			return CONSENT_REFUSAL;
		default:
			return null;
	}
}
