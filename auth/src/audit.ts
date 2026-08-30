import type { Pool } from 'pg';

/**
 * The event vocabulary `db/auth/0002_audit_event.sql` admits. A name added
 * here and not there is rejected by that file's check constraint rather than
 * silently widening the record.
 */
export type AuthEventName =
  | 'user_signed_up'
  | 'user_signed_in'
  | 'user_sign_in_failed'
  | 'user_signed_out'
  | 'password_reset_requested'
  | 'password_reset_completed';

/**
 * One row of the identity audit trail.
 *
 * `identifier` carries the submitted email only where no `userId` is
 * available, which is the failed sign-in and the reset request. Once the
 * subject is known by id, the id is what is recorded: the table is readable
 * by tam_app, and every address written here is one the identity-schema
 * boundary would otherwise have kept from it.
 */
export interface AuthEvent {
  readonly event: AuthEventName;
  readonly userId?: string | undefined;
  readonly identifier?: string | undefined;
  readonly sessionId?: string | undefined;
  readonly ipAddress?: string | undefined;
  readonly userAgent?: string | undefined;
  readonly detail?: string | undefined;
}

// Schema-qualified rather than leaning on the role's search_path: a writer
// that resolves to nothing fails into the catch below, and a silent gap in an
// audit trail is the one failure this table exists to prevent.
const INSERT = `INSERT INTO auth.auth_event
    (event, user_id, identifier, session_id, ip_address, user_agent, detail)
    VALUES ($1, $2, $3, $4, $5, $6, $7)`;

const insert = async (pool: Pool, event: AuthEvent): Promise<void> => {
  await pool.query(INSERT, [
    event.event,
    event.userId ?? null,
    event.identifier ?? null,
    event.sessionId ?? null,
    event.ipAddress ?? null,
    event.userAgent ?? null,
    event.detail ?? null,
  ]);
};

/**
 * Append one event to the audit trail without joining it to the caller's
 * response.
 *
 * Detached for the reason `deliver` in `email.ts` is detached, and with one
 * consequence beyond it: an audit insert that fails must not turn a valid
 * sign-in into an error, so the rejection is logged and the response proceeds.
 * The trade is a loss window -- a row still in flight when the process exits
 * is lost -- accepted because the alternative makes the identity service's
 * availability a function of one table's.
 */
export const record = (pool: Pool, event: AuthEvent): void => {
  void insert(pool, event).catch((cause: unknown) => {
    console.error(`tam-auth: audit insert failed for ${event.event}`, cause);
  });
};
