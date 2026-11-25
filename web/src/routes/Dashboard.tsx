import { useQuery } from '@tanstack/react-query';
import { getJson, type ChainStatus } from '../lib/api';

// First vertical slice: proves web → Caddy → api → Postgres ← indexer end to end.
export function Dashboard() {
  const status = useQuery({ queryKey: ['chain-status'], queryFn: () => getJson<ChainStatus>('/api/chain/status'), refetchInterval: 12_000 });
  return (
    <main className="page">
      <h1 style={{ margin: 0, fontSize: 22, color: 'var(--text-strong)' }}>Portfolio</h1>
      <section className="panel">
        {status.isPending && <p className="dim">Loading…</p>}
        {status.isError && <p role="alert" className="down">Could not reach the API: {String(status.error)}</p>}
        {status.data && (
          <p>
            Indexer at block <span style={{ color: 'var(--text-strong)' }}>#{status.data.live_block?.toLocaleString('en-US') ?? '—'}</span>
            <span className="dim"> · seen {status.data.seen_at ?? 'never'}</span>
          </p>
        )}
      </section>
    </main>
  );
}
