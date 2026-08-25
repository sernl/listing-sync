-- The retention pruner is the engine's, so the one erasure 0007 withheld is
-- granted here: job_event rows past the retention window are deleted and the
-- watermark 0014 added is advanced past them, which is what turns a client's
-- resume below the watermark into a resync rather than a partial replay.
GRANT DELETE ON job_event TO tam_engine;
