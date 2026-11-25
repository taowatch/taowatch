# TaoWatch

Self-hosted, **read-only** tracker for Bittensor wallets: free / stake / claimable / last claim, daily charts
(including whether claimable went up or down), passkey login for several users, realtime updates every block.

- UI design (12 artboards): https://claude.ai/artifact/TMaav81t1MxX3o1ZbPARo2
- Implementation plan and verified chain facts: `PLAN.md` (kept locally, not in git)

```
backend/   Rust workspace — crates/core (config, db, chain), crates/api (axum + WebSocket), crates/indexer (subxt)
           migrations/ (sqlx, TimescaleDB) · metadata/finney.scale (pinned runtime metadata v15) · Dockerfile
web/       React 19 + Vite 8 + TypeScript 7 + Chart.js · Caddyfile · Dockerfile (build → Caddy)
.github/   ci.yml (checks) · images.yml (build & publish Docker images to GHCR)
```

## Images

GitHub Actions publishes two multi-arch (amd64 + arm64) images to GitHub Container Registry:

| image | contains | used by compose service |
|---|---|---|
| `ghcr.io/<owner>/taowatch-backend` | `api` and `indexer` binaries | `api` (`api`), `indexer` (`indexer run`) |
| `ghcr.io/<owner>/taowatch-web` | Caddy + built SPA, HTTPS, reverse proxy | `web` |

Tags: `latest` (main branch) · `sha-<short>` (every build) · `1.2.3` and `1.2` (when you push tag `v1.2.3`).

## CI/CD

| workflow | runs on | does |
|---|---|---|
| `ci.yml` | every push to `main`, every pull request | `cargo fmt --check`, `clippy -D warnings`, `cargo test`; web `typecheck` + `build`; `docker compose config` |
| `images.yml` | push to `main`, tags `v*`, manual run | builds both images natively on `ubuntu-24.04` (amd64) and `ubuntu-24.04-arm` (arm64), pushes by digest, merges multi-arch manifests |

### First-time GitHub setup

1. Create the repository and push:
   ```sh
   git add -A && git commit -m "Initial TaoWatch"
   git remote add origin git@github.com:<owner>/<repo>.git
   git push -u origin main
   ```
2. Nothing else is required for publishing: `images.yml` logs in to GHCR with the built-in `GITHUB_TOKEN`
   (`packages: write`). After the first run the packages appear under your profile → **Packages**.
3. Packages are **private** by default. Either make them public (package → Package settings → Change visibility),
   or log in on the server with a token that has `read:packages`:
   ```sh
   echo <token> | docker login ghcr.io -u <github-user> --password-stdin
   ```
4. The arm64 jobs use GitHub-hosted `ubuntu-24.04-arm` runners. If your plan does not offer them for this repository,
   remove `arm64` from the `arch` matrix in `images.yml`.
5. Release: `git tag v0.1.0 && git push origin v0.1.0` → images tagged `0.1.0` and `0.1`.

## Deploy (server)

Needs Docker with Compose v2, a domain pointing to the server, and ports 80/443 open (Caddy obtains the certificate).

```sh
# on the server: only docker-compose.yml and .env are needed
cp .env.example .env
#   TAOWATCH_IMAGE_PREFIX=ghcr.io/<owner>/taowatch     (lower case)
#   TAOWATCH_TAG=latest                                 (or 0.1.0 to pin a release)
#   DOMAIN=taowatch.example.com
#   RP_ID=taowatch.example.com
#   RP_ORIGIN=https://taowatch.example.com
#   POSTGRES_PASSWORD=…  and the same password inside DATABASE_URL
docker compose pull
docker compose up -d
docker compose logs -f api indexer
```

Update to a new build: `docker compose pull && docker compose up -d`. The API applies database migrations on start.

The first user to register becomes **admin** and can create invite codes for others (Settings → Users & invites).
Passkeys need HTTPS on the real domain; `RP_ID` must equal the domain users open.

Build the images locally instead of pulling: `docker compose up -d --build`.

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

### macOS firewall (Little Snitch, LuLu, …)

Every new Rust build is a new binary. A per-app firewall holds its outbound connections until you allow it, and the
RPC connection then fails with `Connection timeout exceeded: 10s`. Allow `backend/target/*/api` and
`backend/target/*/indexer` (or the whole `backend/target` folder) to connect to `*.opentensor.ai:443`.

## Chain notes

- subxt must use `LegacyBackend`: Subtensor nodes reject the `chainHead_v1` RPCs.
- Stake comes from `StakeInfoRuntimeApi`; claimable from `BetaBasketRuntimeApi` (root claim "beta basket").
- Claimed TAO comes from `BasketClaimed` **events**. The `BasketClaimed` storage is basket bookkeeping that also
  moves on stake changes — it only locates candidate blocks.
- The public live node is pruned; history comes from the archive node, which rate-limits above ~2 concurrent workers.
