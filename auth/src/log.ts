import type { Mode } from './env.ts';

/**
 * The line printed when there is no email transport to hand a message to.
 *
 * The recipient is never printed. It is the one field an operator does not need
 * in order to act on this line, and a log aggregator is the wrong place to
 * accumulate addresses.
 *
 * The action link is a verification or reset token, so it is printed only in
 * development, where this branch *is* the email flow and the link has nowhere
 * else to come from. Production cannot reach this branch at all, because
 * `readRequiredInProduction` makes RESEND_API_KEY mandatory when TAM_AUTH_ENV
 * is production; the production spelling here is a belt over that brace.
 *
 * It takes the two fields rather than a `Delivery` so that no future field can
 * reach a log by being added to that type.
 */
export const transportWarning = (subject: string, url: string, mode: Mode): string => {
  const line = `tam-auth: no email transport configured; not sent; subject ${subject}`;
  return mode === 'development' ? `${line}; link: ${url}` : line;
};
