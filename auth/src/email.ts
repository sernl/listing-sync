import { Resend } from 'resend';
import { env } from './env.ts';

export interface Delivery {
  readonly to: string;
  readonly subject: string;
  readonly lead: string;
  readonly url: string;
}

const resend = env.resendApiKey === undefined ? undefined : new Resend(env.resendApiKey);

const body = (delivery: Delivery): string =>
  [delivery.lead, '', delivery.url, '', 'If you did not request this, ignore this message.'].join('\n');

const send = async (delivery: Delivery): Promise<void> => {
  if (resend === undefined || env.emailFrom === undefined) {
    console.warn(`tam-auth: no email transport configured; ${delivery.subject} link: ${delivery.url}`);
    return;
  }
  const { error } = await resend.emails.send({
    from: env.emailFrom,
    to: delivery.to,
    subject: delivery.subject,
    text: body(delivery),
  });
  if (error !== null) {
    throw new Error(`resend rejected the message: ${error.name}: ${error.message}`);
  }
};

/**
 * Hand a message to the transport without joining it to the caller's response.
 *
 * The send is detached deliberately: awaited inside an auth handler it makes
 * response latency a function of whether the address exists, the enumeration
 * oracle better-auth's own guidance tells integrators to avoid ("Avoid
 * awaiting the email sending to prevent timing attacks",
 * `docs/content/docs/concepts/email.mdx`).
 */
export const deliver = (delivery: Delivery): void => {
  void send(delivery).catch((cause: unknown) => {
    console.error(`tam-auth: delivery failed for ${delivery.subject}`, cause);
  });
};
