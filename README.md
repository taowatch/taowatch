# TaoWatch

Self-hosted, **read-only** tracker for Bittensor wallets: free / stake / claimable / last claim, daily charts
(including whether claimable went up or down), passkey login for several users, realtime updates every block.

```
backend/   Rust workspace — crates/core (config, db, chain), crates/api (axum + WebSocket), crates/indexer (subxt)
           migrations/ (sqlx, TimescaleDB) · metadata/finney.scale (pinned runtime metadata v15) · Dockerfile
web/       React 19 + Vite 8 + TypeScript 7 + Chart.js · Caddyfile · Dockerfile (build → Caddy)
.github/   ci.yml (checks) · images.yml (publish Docker images)
```

## Deploy

Needs Docker with Compose v2 and a reverse proxy that terminates TLS (nginx below). TaoWatch publishes no public
ports: the `web` container listens on `127.0.0.1:8088` (`WEB_BIND`, `WEB_PORT`). Only `docker-compose.yml` and
`.env` are needed on the server; the images are pulled from GitHub Container Registry.

```sh
mkdir taowatch && cd taowatch
curl -fsSLO https://raw.githubusercontent.com/taowatch/taowatch/main/docker-compose.yml
curl -fsSL https://raw.githubusercontent.com/taowatch/taowatch/main/.env.example -o .env
# edit .env:
#   RP_ID=taowatch.example.com                  (the host users open)
#   RP_ORIGIN=https://taowatch.example.com
#   POSTGRES_PASSWORD=…                         (and the same password inside DATABASE_URL)
#   TAOWATCH_DATA_DIR=./data                    (host folder for the database)
#   TAOWATCH_REGISTRATION=open                  (open | invite | closed)
#   TAOWATCH_TAG=latest                         (or a release such as 0.1.0)
docker compose pull
docker compose up -d
docker compose logs -f api indexer
```

Update: `docker compose pull && docker compose up -d`. The API applies database migrations on start.

**Data** is stored in host folders: `${TAOWATCH_DATA_DIR}/postgres` (default `./data/postgres`). Back it up with
`docker compose exec db pg_dump -U taowatch taowatch > backup.sql`, or stop the stack and copy the folder.

### nginx

`/ws` needs the WebSocket upgrade headers; everything else is a plain proxy.

```nginx
server {
    listen 443 ssl;
    http2 on;
    server_name taowatch.example.com;
    # ssl_certificate / ssl_certificate_key …

    location /ws {
        proxy_pass http://127.0.0.1:8088;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 1h;
    }

    location / {
        proxy_pass http://127.0.0.1:8088;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

**Accounts.** Users sign in with **username + password** (Argon2id; 5 failed attempts lock the username for
15 minutes) or a **passkey**, and can use both (Settings → Password / Passkeys). Every account gets six one-time
recovery codes. The first account becomes **admin**. Sign-up is controlled by `TAOWATCH_REGISTRATION`:
`open` (anyone who can reach the site), `invite` (needs an invite code from Settings → Users & invites;
used when the variable is unset) or `closed` (no new accounts). Passkeys only work over HTTPS, and `RP_ID` must equal the host name in the browser.

## CI/CD

- `ci.yml` — on every push to `main` and every pull request: `cargo fmt --check`, `clippy -D warnings`,
  `cargo test`; web `npm ci`, `typecheck`, `build`; `docker compose config`.
- `images.yml` — on push to `main`, on tags `v*`, or run manually: builds and publishes the images used by
  `docker-compose.yml`. Release with `git tag v0.1.0 && git push origin v0.1.0`.

## Develop on the host

```sh
docker compose -f docker-compose.yml -f docker-compose.dev.yml up -d db     # Postgres on 127.0.0.1:5432

cd backend
export DATABASE_URL=postgres://taowatch:<password>@127.0.0.1:5432/taowatch
cargo run --bin api                                   # :8080, applies migrations
cargo run --bin indexer -- run                        # live loop + history backfill

cd ../web && npm install && npm run dev               # :5173, proxies /api and /ws to :8080
```

Requirements: Rust ≥ 1.98 (images use 1.100), Node ≥ 22.22 (images use 26 LTS). Before pushing, run what CI runs:

```sh
cd backend && cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test
cd ../web && npm run typecheck && npm run build
```

### Indexer commands (`cargo run --bin indexer -- …`)

| command | what it does |
|---|---|
| `run` | live loop + backfill workers |
| `track <ss58> [rec\|short\|none]` | track a wallet without a user (testing) |
| `snapshot <ss58> [--archive] [--at <block>]` | print one wallet's state, no DB |
| `events <block> <ss58> [--archive]` | print the wallet's events in a block, no DB |

### Testing the API without a browser

Passkeys need a real authenticator. For local curl/WebSocket tests start the API with
`TAOWATCH_DEV_LOGIN=1 RP_ID=localhost RP_ORIGIN=http://localhost:8080` and call `POST /api/dev/login {"username":"…"}`
(keep the cookie). The route exists only when all three conditions hold, logs a warning, and can never be enabled behind HTTPS.

### Troubleshooting

`Connection timeout exceeded: 10s` when connecting to `*.opentensor.ai`: check that outbound WebSocket (port 443)
is allowed for the binary. Per-application firewalls treat every new build as a new program.

## Chain notes

- subxt must use `LegacyBackend`: Subtensor nodes reject the `chainHead_v1` RPCs.
- Stake comes from `StakeInfoRuntimeApi`; claimable from `BetaBasketRuntimeApi` (root claim "beta basket").
- Claimed TAO comes from `BasketClaimed` **events**. The `BasketClaimed` storage is basket bookkeeping that also
  moves on stake changes — it only locates candidate blocks.
- The TAO price comes from Binance's public `taousdt@ticker` WebSocket (no key); the status bar updates at most
  every 5 s and a sample is stored every 15 minutes. `TAOWATCH_PRICE=off` disables it, `TAOWATCH_PRICE_WS` changes the URL.
- The public live node is pruned; history comes from the archive node, which rate-limits above ~2 concurrent workers.
