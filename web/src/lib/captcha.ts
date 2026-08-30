// The captcha gate as the client sees it.
//
// `tam-auth` mounts better-auth's captcha plugin only when it was given a
// Turnstile secret key (`auth/src/auth.ts`), and passes no `endpoints`
// override, so the plugin guards exactly its three defaults: `/sign-in/email`,
// `/sign-up/email` and `/request-password-reset`. Those are the only three
// client calls that carry the header; passkey and social sign-in are
// deliberately not among them.
//
// A build with no site key has to produce requests indistinguishable from the
// ungated ones. That is why `captchaOptions` returns `undefined` rather than
// empty headers: better-auth's client proxy reads its second argument as
// `args[1] || {}`, so an omitted and an `undefined` one are the same request,
// while an empty header object is a different one.

/** The header better-auth's captcha plugin reads off the request. */
export const CAPTCHA_HEADER = 'x-captcha-response';

/** Fetch options carrying a solved challenge, in the shape better-auth's
 * client takes as its second argument. */
export interface CaptchaOptions {
	headers: Record<string, string>;
}

/** A site key is configured or it is not; blank and whitespace count as not. */
export function readSiteKey(raw: unknown): string | null {
	if (typeof raw !== 'string') {
		return null;
	}
	const trimmed = raw.trim();
	return trimmed.length > 0 ? trimmed : null;
}

/** Substituted by Vite at build time. Absent from every build that does not
 * define it, which is what keeps the widget and its script dormant. */
export const TURNSTILE_SITE_KEY = readSiteKey(import.meta.env.VITE_TURNSTILE_SITE_KEY);

export function captchaOptions(token: string | null): CaptchaOptions | undefined {
	return token === null ? undefined : { headers: { [CAPTCHA_HEADER]: token } };
}

/** Whether a form must wait: a challenge is configured and not yet solved. */
export function captchaPending(siteKey: string | null, token: string | null): boolean {
	return siteKey !== null && token === null;
}
