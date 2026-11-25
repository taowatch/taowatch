-- create_hypertable already indexes (ts DESC); the extra index from 0003 was a duplicate.
DROP INDEX IF EXISTS prices_ts;
