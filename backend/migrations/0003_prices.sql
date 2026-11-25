-- TAO/USDT price samples (Binance ticker stream, stored every 15 min).
CREATE TABLE prices (
    ts         timestamptz NOT NULL,
    tao_usd    numeric(20,8) NOT NULL,
    change_24h numeric(10,4),
    source     text NOT NULL DEFAULT 'binance'
);
SELECT create_hypertable('prices', by_range('ts', INTERVAL '30 days'));
CREATE INDEX prices_ts ON prices (ts DESC);
