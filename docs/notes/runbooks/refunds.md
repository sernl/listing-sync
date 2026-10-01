# Refunds: issuing one from the Payments page

- date: 2026-10-02
- status: in use from 0.18.0
- companions: `operator-markings.md` (who can open `/admin`), `crates/tam-api/src/payments.rs`, migration `0102_payments.sql`

## Where

Admin → **Payments** (`/admin/payments`), next to Pricing. Only a platform operator reaches it.
Every row is one money event from Stripe: a payment, a failed payment, a refund, a dispute (shown as **Chargeback**), a paid or failed invoice, a subscription started or cancelled.
Open a row to see everything else about the same charge or invoice: its refunds, its dispute, the invoice it paid.

If the page looks thin (a new deployment, or money that moved before the webhook listened for these events), press **Sync from Stripe**.
It reads the last 90 days of charges, refunds, disputes and invoices and fills the gaps. Running it twice changes nothing.

## Issuing a refund

1. Find the payment (filter by kind **Payments**, search the organisation, or narrow the dates) and press **Refund** on its row.
   A row with nothing left to refund shows **Refunded** and has no Refund button.
2. The amount is filled in with what is left on the charge. Lower it for a partial refund.
   The server checks the charge in Stripe at the moment you confirm: Stripe's own total, including any refund made in Stripe's dashboard, decides what is left.
   Asking for more answers "You can refund at most $X on this payment."; a charge refunded in full answers "This payment has already been refunded in full."
3. Pick the reason Stripe records: **Duplicate**, **Fraudulent** or **Requested by the customer**. Fraudulent also tells Stripe's fraud tools about the card.
4. Write a note if it helps the next person (it stays on our side; the customer never sees it).
5. **Email the customer** is ticked when the page's "Email customers when I refund" switch is on. Untick it to stay quiet.
6. Confirm the sentence. The toast says "Refunded $X." and the row shows the refund as **Pending** until Stripe settles it (usually seconds; `refund.updated` moves it to **Succeeded**, or **Failed** if the bank refused).

Pressing confirm twice, or a dropped connection, never refunds twice: the panel sends one id per refund, and Stripe gets it as the idempotency key.
Every refund issued here writes a line in `platform_operator_event` (action `refund`, the operator's id, the refund's id).

## What the customer sees

- Their card statement: the refund, 5–10 business days later depending on the bank. Stripe shows it as pending until the bank takes it.
- If the mail was sent, one email from our seller-facing address to every person in the organisation with a verified address:

  > Subject: We've refunded $12.00
  >
  > Kia ora Aroha,
  >
  > We've refunded $12.00 to your card. It can take 5–10 business days to show.
  >
  > This is for Kauri Room. If anything looks wrong, reply to this email and I'll sort it out.
  >
  > [See your bills]

- Nothing else: the reason and the note are ours.

## The mail afterwards

A refund issued without the mail shows **Send email** in the row's expansion. Pressing it queues the same mail.
Once it goes out, the button reads **Sent on 3 October 2026** and is disabled; asking again answers that sentence.
"Queued" means the server has not sent it yet. On a deployment without `--resend-api-key-file` nothing sends, and queued mails wait.
If a send fails (nobody in the organisation has a verified address, or the mail relay refused it), the reason shows under the button and **Send email** retries.
A payment no organisation can be matched to has nobody to email; the button says so.

The **Email customers when I refund** switch also covers refunds made in Stripe's dashboard: with it on, the webhook queues their mail too. A sync never mails anybody.

## Deploy prerequisites

- The Stripe webhook endpoint (`/v1/billing/webhook`) must be subscribed to these events besides the existing five: `charge.succeeded`, `charge.failed`, `refund.created`, `refund.updated`, `refund.failed`, `charge.dispute.created`, `charge.dispute.closed`, `customer.subscription.created`.
- The Stripe key at `--stripe-secret-key <path>` needs **write** on Refunds and **read** on Charges, Disputes and Invoices (a restricted key made for checkout alone will answer 403 on the sync and the refund).
