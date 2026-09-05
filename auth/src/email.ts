import { Resend } from 'resend';
import { env } from './env.ts';
import { transportWarning } from './log.ts';
import { type Delivery, plainText, render } from './template.ts';

const resend = env.resendApiKey === undefined ? undefined : new Resend(env.resendApiKey);

const send = async (delivery: Delivery): Promise<void> => {
  if (resend === undefined || env.emailFrom === undefined) {
    console.warn(transportWarning(delivery.subject, delivery.url, env.mode));
    return;
  }
  const { error } = await resend.emails.send({
    from: env.emailFrom,
    to: delivery.to,
    subject: delivery.subject,
    html: render(delivery, env.baseUrl),
    text: plainText(delivery, env.baseUrl),
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
