import { Link } from 'react-router';
import { useQuery } from '@tanstack/react-query';
import { getJson } from '../lib/api';
import { num, short, signed, tao, utc } from '../lib/format';
import type { WalletRow } from '../lib/types';
import { Empty, ErrorBox, Loading, Page } from '../components/ui';

export function Wallets() {
  const q = useQuery({ queryKey: ['wallets'], queryFn: () => getJson<WalletRow[]>('/api/wallets') });
  return (
    <Page title="Wallets" actions={<Link to="/onboarding" className="btn btn-link btn-accent"><kbd>a</kbd> Add wallet</Link>}>
      {q.isPending ? <Loading /> : q.isError ? <ErrorBox error={q.error} /> : q.data.length === 0 ? (
        <Empty>
          <p className="m0">No wallets yet.</p>
          <Link to="/onboarding" className="btn-primary btn-link">Add your first wallet</Link>
        </Empty>
      ) : (
        <div className="cards">
          {q.data.map((w) => {
            const ch = signed(w.change_24h);
            const total = (num(w.free) ?? 0) + (num(w.stake) ?? 0) + (num(w.claimable) ?? 0);
            return (
              <Link key={w.address} to={`/wallets/${w.address}`} className="card">
                <span className="card-head"><span className="strong">{w.label} ›</span><span className={ch.tone}>{ch.text}</span></span>
                <span className="dim small">{short(w.address)}{w.history_status === 'running' || w.history_status === 'pending' ? ` · syncing ${w.history_progress}%` : ''}</span>
                <span className="kv">
                  <span className="dim">total</span><span className="r strong">{tao(total)}</span>
                  <span className="dim">stake</span><span className="r stake">{tao(w.stake)}</span>
                  <span className="dim">free</span><span className="r">{tao(w.free, 4)}</span>
                  <span className="dim">claimable</span><span className="r claimable">{tao(w.claimable, 4)}</span>
                  <span className="dim">last claim</span><span className="r">{w.last_claim_at ? utc(w.last_claim_at).slice(0, 10) : 'never'}</span>
                </span>
              </Link>
            );
          })}
        </div>
      )}
    </Page>
  );
}
