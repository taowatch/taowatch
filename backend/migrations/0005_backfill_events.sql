-- History backfill now also recovers stake / unstake / move / transfer events (by bisecting the
-- coldkey nonce). Run it once more for wallets whose history was already loaded: snapshot points
-- already stored are skipped, so only the event search does real work.
UPDATE backfill_jobs SET status = 'queued', updated_at = now() WHERE status IN ('done', 'error') AND plan <> 'none';
