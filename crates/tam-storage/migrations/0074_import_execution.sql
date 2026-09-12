-- Who is executing an import, under what fence, and what it has actually
-- reported.
--
-- 0070 gave an import a run and a pause. What it did not give it was an
-- owner: the run said `reading` from the moment a seller pressed the button,
-- and nothing in the row could tell "a device is walking the shop" from "no
-- device ever picked this up", from "the phone that had it has been off since
-- Tuesday". The console rendered the same sentence for all three, the
-- scheduler's finish step could not tell a dead pass from a slow one, and a
-- device that had been taken over could still post a page, because there was
-- no fence for a page to fail.
--
-- So the run gains the facts an owner has: which device holds it, under which
-- attempt, until when, when it last spoke, when it last made progress, what
-- it last said it was doing and why it stopped. Every one of them is a stored
-- fact rather than an inference: a browser watching an event stream cannot
-- tell whether the machine on the other side is alive, and this migration is
-- what makes the server able to say so instead.
--
-- Two rules the columns exist to state.
--
-- The fence is the attempt. A claim raises it, and a page, a renewal or a
-- progress report naming an older one is refused before it writes anything.
-- That is why `attempt` is `NOT NULL` with a zero default rather than
-- nullable: zero is "never claimed", and an unfenced page for a catalogue run
-- has no attempt to be equal to.
--
-- Authorisation is not a state. `committing` used to be the whole of "the
-- seller asked for these resources to be created", which meant a description
-- pass that finished authorised a catalogue addition nobody confirmed. The
-- confirmation is now its own fact, with the actor and the instant beside it,
-- and a scheduled run carries its own separately approved rule instead — so
-- an old `committing` row backfilled by this migration authorises nothing.

ALTER TABLE import_run
    -- The machine that holds this run, and NULL where none does. Compound
    -- against `device` rather than by bare id, in the shape every other
    -- device-scoped reference in this schema takes: a device id is unique
    -- inside one organisation and nowhere else.
    ADD COLUMN owner_device         text,
    -- The fence. Raised by every claim, including a reclaim by the same
    -- owner, so a page in flight from a previous attempt cannot land after a
    -- resume.
    ADD COLUMN attempt              bigint      NOT NULL DEFAULT 0,
    -- When this owner's hold lapses, by the database's own clock. Compared
    -- against `now()` in the statement that reads it rather than against a
    -- server's wall clock, because the two servers that could disagree are
    -- exactly the ones a takeover decides between.
    ADD COLUMN lease_expires_at     timestamptz,
    -- The last time the owner spoke at all: a renewal, a progress report or a
    -- page. Freshness, which is not progress.
    ADD COLUMN last_contact_at      timestamptz,
    -- The last time something actually moved. A device that renews politely
    -- for an hour while enumerating nothing is contactable and stuck, and the
    -- console has to be able to say which.
    ADD COLUMN last_progress_at     timestamptz,
    -- What the owner last said it was doing, in its own vocabulary. The
    -- displayed stage is this combined with freshness and the run's state by
    -- the server; the browser is never asked to infer it.
    ADD COLUMN reported_stage       text,
    -- Why it stopped, as a closed code plus the bounded sentence beside it.
    -- The code is what the console renders a next action from; the sentence
    -- is the device's own words and carries no marketplace response.
    ADD COLUMN reason_code          text,
    ADD COLUMN reason               text,
    -- What the owner has discovered and processed. Accepted counts never
    -- regress: a progress report carrying a smaller number than the one
    -- already accepted is a report from a device that restarted its own
    -- counter, not a shop that shrank.
    ADD COLUMN discovered           int         NOT NULL DEFAULT 0,
    ADD COLUMN processed            int         NOT NULL DEFAULT 0,
    -- Whether discovery is closed. A listed page may now be partial, so the
    -- presence of `read_total` no longer answers this: only this bit does,
    -- and only the device setting it closes the enumeration.
    ADD COLUMN enumeration_complete boolean     NOT NULL DEFAULT false,
    -- The denominator the reading stage is measured against, frozen when the
    -- selection is taken. A progress bar whose denominator moves under it is
    -- the defect this column exists to stop.
    ADD COLUMN selected_total       int,
    -- The seller's own confirmation that these resources are to be created,
    -- with the actor beside the instant in 0045's voice: an instant nobody
    -- attributed is not a confirmation.
    ADD COLUMN commit_authorised_at timestamptz,
    ADD COLUMN commit_authorised_by text,
    -- The settled run this one retries, where a seller explicitly asked to
    -- try again.
    --
    -- A link rather than a reopening, which is design 170's rule: a terminal
    -- run stays terminal and keeps what it recorded, and the retry is a new
    -- run that names its parent so the console can draw the lineage. Never
    -- set by the scheduler, by a spreadsheet batch or by an ordinary start:
    -- there is no automatic retry here, only a seller pressing Retry on a run
    -- that has settled.
    ADD COLUMN retry_of             uuid,

    ADD CONSTRAINT import_run_owner_device
        FOREIGN KEY (org_id, owner_device) REFERENCES device (org_id, id),
    ADD CONSTRAINT import_run_attempt_non_negative CHECK (attempt >= 0),
    -- An owner and a fence are one fact. Attempt zero is "never claimed", and
    -- a claimed run keeps its owner when its lease lapses -- that is what
    -- makes "the phone that had this is not answering" a statement the
    -- console can make.
    ADD CONSTRAINT import_run_owner_total CHECK ((owner_device IS NULL) = (attempt = 0)),
    ADD CONSTRAINT import_run_lease_follows_owner CHECK (
        lease_expires_at IS NULL OR owner_device IS NOT NULL
    ),
    ADD CONSTRAINT import_run_reported_stage CHECK (
        reported_stage IS NULL
        OR reported_stage IN ('discovering', 'selecting', 'reading', 'interrupted', 'failed')
    ),
    ADD CONSTRAINT import_run_reason_code CHECK (
        reason_code IS NULL
        OR reason_code IN ('missing_session', 'not_permitted', 'unsupported_source',
                           'enumeration_failed', 'description_failed', 'submission_failed',
                           'activation_expired', 'lease_expired', 'stopped',
                           'client_update_required')
    ),
    -- A sentence with no code is prose nothing can act on.
    ADD CONSTRAINT import_run_reason_needs_code CHECK (reason IS NULL OR reason_code IS NOT NULL),
    ADD CONSTRAINT import_run_reason_bounded CHECK (
        reason IS NULL OR char_length(reason) <= 500
    ),
    ADD CONSTRAINT import_run_reported_counts CHECK (discovered >= 0 AND processed >= 0),
    ADD CONSTRAINT import_run_selected_total_non_negative CHECK (
        selected_total IS NULL OR selected_total >= 0
    ),
    ADD CONSTRAINT import_run_commit_authorisation_total CHECK (
        (commit_authorised_at IS NULL) = (commit_authorised_by IS NULL)
    ),
    ADD CONSTRAINT import_run_commit_authorised_by_bounded CHECK (
        commit_authorised_by IS NULL OR char_length(commit_authorised_by) BETWEEN 1 AND 200
    );

ALTER TABLE import_run
    ADD CONSTRAINT import_run_retry_of
        FOREIGN KEY (org_id, retry_of) REFERENCES import_run (org_id, id),
    -- A run does not retry itself.
    ADD CONSTRAINT import_run_retry_of_other CHECK (retry_of IS NULL OR retry_of <> id);

CREATE INDEX import_run_by_retry_of ON import_run (org_id, retry_of)
    WHERE retry_of IS NOT NULL;

-- The backfill, and what it deliberately does not claim.
--
-- No ownership and no lease: nothing in the old schema recorded which device
-- was reading, so every existing run reads as unclaimed and the maintenance
-- pass treats a manual one as interrupted rather than as work in flight. An
-- old run with zero pages therefore becomes visibly unclaimed, which is the
-- truth, instead of a run that looks like it is still going.
--
-- Discovery and progress are backfilled from rows that exist rather than from
-- a guess: `read_total` is written by the list and by nothing else, so its
-- presence is exactly "the enumeration closed" under the old protocol, and
-- the two counts are a count of the run's own items.
UPDATE import_run r
   SET enumeration_complete = (r.read_total IS NOT NULL),
       discovered = COALESCE((
           SELECT count(*)::int FROM import_run_item i
            WHERE i.org_id = r.org_id AND i.run_id = r.id
       ), 0),
       processed = COALESCE((
           SELECT count(*)::int FROM import_run_item i
            WHERE i.org_id = r.org_id AND i.run_id = r.id
              AND i.state IN ('read', 'matched', 'review', 'imported', 'skipped', 'failed')
       ), 0);

-- One open run per source, rather than one per organisation.
--
-- The organisation-wide fence was the reason a seller reading Tes could not
-- start TPT, and the reason a scheduled pull for one shop waited a pass for
-- the other. It is replaced by the two fences that are actually meant: one
-- open run per marketplace source, and one open run per spreadsheet batch.
-- Strictly weaker than what it replaces, so no existing row can violate it.
DROP INDEX import_run_one_open_per_org;

CREATE UNIQUE INDEX import_run_one_open_per_source ON import_run (org_id, source)
    WHERE source IS NOT NULL AND state IN ('reading', 'reviewing', 'committing');

CREATE UNIQUE INDEX import_run_one_open_per_batch ON import_run (org_id, batch_id)
    WHERE batch_id IS NOT NULL AND state IN ('reading', 'reviewing', 'committing');

-- The runs a device or the drain has to find, which is a per-source read
-- rather than a singleton lookup.
CREATE INDEX import_run_open_by_source ON import_run (org_id, source, state)
    WHERE state IN ('reading', 'reviewing', 'committing');

-- What a seller's own press of the button was, so pressing it twice is one
-- import.
--
-- The key is minted by the client and retained across its retries and across
-- a native handoff; only an explicit new import intent mints another. The
-- binding survives the run settling, which is the whole point: a lost
-- acknowledgement replayed after the run completed or after its activation
-- expired must reach the run it already made rather than mint a second one.
CREATE TABLE import_run_start_key (
    org_id     uuid        NOT NULL REFERENCES organisation (id),
    start_key  uuid        NOT NULL,
    -- The shop the key was spent on. A replay naming a different source is a
    -- different intent wearing an answered key, and is refused rather than
    -- silently answered with the first run.
    source     text        NOT NULL,
    run_id     uuid        NOT NULL,
    -- The settled run the intent this key was spent on was retrying, where it
    -- was retrying one. Part of the key's identity beside the source: "start
    -- Tes" and "retry that failed Tes run" are different intents, so a replay
    -- of one key carrying the other is refused rather than answered with the
    -- wrong run.
    retry_of   uuid,
    created_at timestamptz NOT NULL,

    PRIMARY KEY (org_id, start_key),
    FOREIGN KEY (org_id, run_id) REFERENCES import_run (org_id, id)
);

CREATE INDEX import_run_start_key_by_run ON import_run_start_key (org_id, run_id);

-- One page's acknowledgement, kept so a replay is answered rather than
-- applied twice.
--
-- The receipt is the device's own logical key for a page, and `identity` is
-- the digest of what that page carried. The attempt is deliberately excluded
-- from the identity: a device that resumed under a new fence and resent the
-- page it never got an answer for is sending the same page, and telling it
-- otherwise would cost the seller those resources twice. The same key with
-- different content is the opposite -- a client bug or a modified device --
-- and conflicts.
--
-- The acknowledgement itself is stored rather than recomputed, because a
-- replay has to be answered with what the first delivery was told: counts
-- recomputed later would report zero applied for a page that applied
-- fourteen, and the device reads `applied` to decide whether it has to resend.
CREATE TABLE import_run_receipt (
    org_id          uuid        NOT NULL REFERENCES organisation (id),
    run_id          uuid        NOT NULL,
    receipt         uuid        NOT NULL,
    identity        bytea       NOT NULL,
    applied         int         NOT NULL,
    skipped         int         NOT NULL,
    described_total int         NOT NULL,
    complete        boolean     NOT NULL,
    accepted_at     timestamptz NOT NULL,

    PRIMARY KEY (org_id, run_id, receipt),
    FOREIGN KEY (org_id, run_id) REFERENCES import_run (org_id, id),

    CONSTRAINT import_run_receipt_identity_width CHECK (octet_length(identity) = 32),
    CONSTRAINT import_run_receipt_counts CHECK (
        applied >= 0 AND skipped >= 0 AND described_total >= 0
    )
);

ALTER TABLE import_run_start_key ENABLE ROW LEVEL SECURITY;
ALTER TABLE import_run_start_key FORCE ROW LEVEL SECURITY;
CREATE POLICY import_run_start_key_org_isolation ON import_run_start_key
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE import_run_receipt ENABLE ROW LEVEL SECURITY;
ALTER TABLE import_run_receipt FORCE ROW LEVEL SECURITY;
CREATE POLICY import_run_receipt_org_isolation ON import_run_receipt
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);
