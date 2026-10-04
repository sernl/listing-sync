# Refunds: the policy, the Payments page, and sellers' requests

- date: 2026-10-05
- status: in use from 0.18.0; the policy calculator and "Ask for a refund" from 0.21.0
- companions: `operator-markings.md` (who can open `/admin`), `crates/tam-api/src/refund_policy.rs` (the policy), `crates/tam-api/src/refund_quote.rs` (charge → purchase), `crates/tam-api/src/payments.rs`, `crates/tam-api/src/refund_requests.rs`, migrations `0102_payments.sql` and `0107_refund_policy.sql`, Terms `teachouse.io/terms/#refunds`

## The policy, as the code applies it

The Terms' refunds section is the rule; `refund_policy.rs` is it in code, with a unit test per edge.

- **Move Pack:** refunded in full if none of its moves has been used and it was bought less than 14 days ago. The 14 days end at the same time of day 14 New Zealand days after the purchase (day 15 is too late), across daylight saving.
- **Monthly plan:** a month that has started is not refunded. Cancelling stops the next renewal.
- **Yearly plan:** `refund = yearly price × max(0, unused whole months − 1) ÷ 12`, rounded down to the cent.
  Months are New Zealand (`Pacific/Auckland`) calendar months counted from the period start: month *k* starts on the same day and time *k − 1* months later (the last day of a shorter month when the day is missing). A month is used from the instant it starts, so on day 1 eleven months are unused and the refund is 10/12; from month 11 there is nothing left. The plan ends on the day of the refund.

The price is what the charge took (tax and any discount included), because that is what goes back. The quote never exceeds what Stripe says is left on the charge, so an earlier refund (ours or the dashboard's) counts against it.

### How a charge is traced to what it bought

A charge names neither a plan nor a pack, so the quote traces it:

1. The invoice it paid: named on the charge (API versions before 2025-03-31), found in our ledger's `invoice_paid` rows by charge or payment intent, or asked of Stripe's invoice payments. The invoice line gives the price (so plan and cadence) and the period the months are counted in.
2. Otherwise the Checkout Session its payment intent completed, whose line items name the pack. "Moves used" is the most the move ledger can prove: commits stamped with the pack's expiry, or the shortfall when the balance has fallen below the pack's size.
3. Neither: **Not matched to a plan or pack**, quoted at $0. Refund by judgement and say why.

Known limits: a proration invoice (a plan switch mid-year) counts months from its own line period, and a pack bought before `checkout.session.completed` reached us has no ledger credit yet and reads as not matched. Override with a reason in both cases.

## Where

Admin → **Payments** (`/admin/payments`), next to Pricing. Only a platform operator reaches it.
Every row is one money event from Stripe: a payment, a failed payment, a refund, a dispute (shown as **Chargeback**), a paid or failed invoice, a subscription started or cancelled.
Open a row to see everything else about the same charge or invoice: its refunds, its dispute, the invoice it paid.

If the page looks thin (a new deployment, or money that moved before the webhook listened for these events), press **Sync from Stripe**.
It reads the last 90 days of charges, refunds, disputes and invoices and fills the gaps. Running it twice changes nothing.

## Issuing a refund

1. Find the payment (filter by kind **Payments**, search the organisation, or narrow the dates) and press **Refund** on its row.
   A row with nothing left to refund shows **Refunded** and has no Refund button.
2. The panel asks the server for the policy quote (`GET /v1/admin/payments/charges/{charge}/quote`) and fills in the amount from it. The **Policy** block names what was bought, the rule, and the sentence that explains the amount, for example:
   > This is month 4 of the yearly plan, so months 5 to 12 are unused: 8 whole months. Less one month's fee, the refund is 7 × $240.00 ÷ 12 = $140.00.
3. To refund a different amount, change it and fill in **Why a different amount?** The server refuses an amount other than the quote without a reason ("Say why you're refunding $X rather than the policy's $Y."). The reason is stored with the refund.
   The server re-quotes at the moment you confirm. If the quote moved since the panel opened (a month turned over, a refund landed in the dashboard) it answers "The policy amount for this payment changed since the panel opened: it is now $X." and the panel reloads the quote.
   Asking for more than is left answers "You can refund at most $X on this payment."; a charge refunded in full answers "This payment has already been refunded in full."
4. For a yearly plan, **End the plan today** is ticked: the Terms say the plan ends on the day of the refund. Confirming cancels the subscription in Stripe immediately, ends the plan grant now, and the account is on Look. Untick it only when the customer keeps the plan (a goodwill partial refund).
5. Pick the reason Stripe records: **Duplicate**, **Fraudulent** or **Requested by the customer**. Fraudulent also tells Stripe's fraud tools about the card.
6. Write a note if it helps the next person (it stays on our side; the customer never sees it).
7. **Email the customer** is ticked when the page's "Email customers when I refund" switch is on. Untick it to stay quiet.
8. Confirm the sentence. The toast says "Refunded $X." and the row shows the refund as **Pending** until Stripe settles it.

A Move Pack refunded in full also takes its moves back out of the balance (a `refund` row in `move_ledger`, once per pack).
Pressing confirm twice, or a dropped connection, never refunds twice: the panel sends one id per refund, and Stripe gets it as the idempotency key.
Every refund issued here writes a line in `platform_operator_event` (action `refund`), and the `refund` row keeps `policy_basis`, `quoted_cents` and `override_reason` beside `amount_cents`, so the audit reads "policy said X, we refunded Y, because Z".

## Sellers asking for a refund

Billing (`/settings/billing`) has **Ask for a refund**, with the policy behind **Explain**. The seller picks one of their payments from the last 400 days, sees the policy's sentence and amount, adds a note if they like, and presses **Ask for $X**. The amount is always the server's quote.
A payment the policy refunds nothing on cannot be asked about (the seller sees the policy's sentence). One request waits per payment.

Every operator with a verified address gets a mail (or the operations inbox, `--ops-email`, when none has one):

> Subject: Refund request: $140.00 for Kauri Room
>
> A seller asked for a refund. Approve or decline it on the Payments page, or reply to this mail to talk to them.
> Name / Email / Organisation / Policy amount / Why (the policy sentence) / Payment / Their note
>
> [Open Payments]

On the Payments page, **Refund requests** lists them, waiting ones first.

- **Approve** refunds the amount quoted on the day they asked (not today's, which may be lower; the refund records that as its reason), emails the customer the refund mail, and for a yearly plan ends the plan today. Retrying an approval answers the same refund.
- **Decline** needs a reason. The seller gets:
  > Subject: About your refund request
  >
  > Kia ora Aroha,
  >
  > I looked at your request for a $29.00 refund, and I can't refund it this time.
  >
  > (your reason)
  >
  > This is for Kauri Room. If you'd like to talk it through, reply to this email.

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

- A yearly plan refunded with **End the plan today**: Billing shows Look from that moment.
- Nothing else: the reason, the note and the override reason are ours.

## The mail afterwards

A refund issued without the mail shows **Send email** in the row's expansion. Pressing it queues the same mail.
Once it goes out, the button reads **Sent on 3 October 2026** and is disabled; asking again answers that sentence.
"Queued" means the server has not sent it yet. On a deployment without `--resend-api-key-file` nothing sends, and queued mails wait.
If a send fails (nobody in the organisation has a verified address, or the mail relay refused it), the reason shows under the button and **Send email** retries.
A payment no organisation can be matched to has nobody to email; the button says so.

The **Email customers when I refund** switch also covers refunds made in Stripe's dashboard: with it on, the webhook queues their mail too. A sync never mails anybody.

## Deploy prerequisites

- Migration `0107_refund_policy.sql` (adds the quote columns to `refund`, and `refund_request`).
- The Stripe webhook endpoint (`/v1/billing/webhook`) must be subscribed to these events besides the existing five: `charge.succeeded`, `charge.failed`, `refund.created`, `refund.updated`, `refund.failed`, `charge.dispute.created`, `charge.dispute.closed`, `customer.subscription.created`. The seller's list of payments is read from `charge.succeeded` rows.
- The Stripe key at `--stripe-secret-key <path>` needs **write** on Refunds and Subscriptions (an approved yearly refund cancels the subscription), and **read** on Charges, Disputes, Invoices, Invoice Payments and Checkout Sessions (the quote's trace).
