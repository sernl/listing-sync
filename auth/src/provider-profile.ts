/**
 * What a social sign-in says about its own address.
 *
 * Founder decision A1 (2026-09-07): an address Google or Microsoft supplied at
 * sign-in counts as verified. better-auth 1.7.2 fills `emailVerified` from the
 * provider's own claim, and Microsoft Entra sends `email_verified` only as an
 * optional claim an app registration has to be configured for, so its provider
 * defaults the field to false
 * (`@better-auth/core/dist/social-providers/microsoft-entra-id.mjs`); a seller
 * who signed in with Microsoft would then hold an address this platform never
 * mails. Both providers spread `mapProfileToUser`'s answer over their own, so
 * what this returns is the last word.
 *
 * A profile carrying no address stays unverified: there is nothing to vouch
 * for, and better-auth refuses the sign-in on the missing address anyway.
 */
export interface ProviderProfile {
  readonly email?: string | undefined;
}

export const vouchedByProvider = (profile: ProviderProfile): { emailVerified: boolean } => ({
  emailVerified: typeof profile.email === 'string' && profile.email.trim() !== '',
});
