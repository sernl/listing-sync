/**
 * The WebAuthn relying party a passkey is bound to, when TAM_AUTH_PASSKEY_RP_ID
 * does not name one.
 *
 * The console lives on `dash.teachouse.io` and the marketing site on the apex,
 * and a passkey registered against the apex works on every subdomain of it,
 * so the default is the registrable domain of the base URL's host rather than
 * the host itself: a later move of the console to another subdomain keeps
 * every seller's passkeys working.
 *
 * The registrable domain is taken as the last two labels. That is right for
 * `teachouse.io` and wrong under a two-label public suffix such as `co.nz`,
 * where the browser would refuse the suffix as a relying party; a deployment
 * there sets TAM_AUTH_PASSKEY_RP_ID explicitly. A loopback name or an IP
 * address has no parent to climb to and is its own relying party.
 */
export const defaultPasskeyRpId = (hostname: string): string => {
  if (hostname.startsWith('[') || /^\d{1,3}(\.\d{1,3}){3}$/u.test(hostname)) {
    return hostname;
  }
  const labels = hostname.split('.');
  return labels.length <= 2 ? hostname : labels.slice(-2).join('.');
};
