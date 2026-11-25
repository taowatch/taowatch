import { useQuery, useQueryClient } from '@tanstack/react-query';
import { NavLink, useNavigate } from 'react-router';
import { getJson, netState, type ChainStatus } from '../lib/api';
import { logout } from '../lib/passkey';
import { useMe } from './RequireAuth';

const TABS = [
  ['/', '1', 'dashboard'],
  ['/wallets', '2', 'wallets'],
  ['/claimable', '3', 'claimable'],
  ['/activity', '4', 'activity'],
  ['/alerts', '5', 'alerts'],
  ['/settings', '6', 'settings'],
] as const;

// Shown only when balances are not current; nothing is shown while data is live.
const STALE = {
  lag: { icon: '◐', color: 'var(--warn)', label: (b: string) => `data delayed · last block #${b}` },
  offline: { icon: '○', color: 'var(--down)', label: (b: string) => `data source offline · last block #${b}` },
} as const;

type Price = { usd: string; change_24h: string | null; ts: string } | null;

/** No sample for this long means the price feed is down (rows are stored every 15 min). */
const PRICE_STALE_MS = 35 * 60_000;
const newer = (a: Price | undefined, b: Price) => (a && (!b || Date.parse(a.ts) > Date.parse(b.ts)) ? a : b);

export function StatusBar() {
  const { data } = useQuery({ queryKey: ['chain-status'], queryFn: () => getJson<ChainStatus>('/api/chain/status'), refetchInterval: 12_000 });
  const state = netState(data);
  const block = data?.live_block?.toLocaleString('en-US') ?? '—';
  const stale = state === 'lag' || state === 'offline' ? STALE[state] : null;
  const qc = useQueryClient();
  // The WebSocket writes fresher prices into this cache; a refetch (newest stored row, up to
  // 15 min old) must not replace them.
  const price = useQuery({
    queryKey: ['price'],
    queryFn: async () => newer(qc.getQueryData<Price>(['price']), await getJson<Price>('/api/price')),
    refetchInterval: 60_000,
  });
  const usd = price.data ? Number(price.data.usd) : null;
  const ch = price.data?.change_24h != null ? Number(price.data.change_24h) : null;
  const priceStale = price.data ? Date.now() - Date.parse(price.data.ts) > PRICE_STALE_MS : false;
  const me = useMe();
  const navigate = useNavigate();
  const signOut = async () => {
    await logout();
    qc.clear();
    navigate('/login', { replace: true });
  };
  return (
    <header className="top">
      <div className="top-in">
        <NavLink to="/" className="brand">[taowatch]</NavLink>
        <nav aria-label="Main" className="tabs">
          {TABS.map(([to, key, label]) => (
            <NavLink key={to} to={to} end={to === '/'} className="tab">
              <span className="tab-key">{key}</span>
              {label}
            </NavLink>
          ))}
        </nav>
        {stale && (
          <span role="status" className="top-stale" style={{ borderColor: stale.color, color: stale.color }}>
            <span aria-hidden="true">{stale.icon}</span>
            {stale.label(block)}
          </span>
        )}
        <span className="top-price" title={price.data ? `Binance TAO/USDT · updated ${new Date(price.data.ts).toLocaleString()}` : 'TAO price unavailable'}>
          <span className="dim">TAO </span>
          <span className={priceStale ? 'dim' : 'strong'}>{usd !== null ? `$${usd.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}` : '—'}</span>
          {ch !== null && <span className={ch >= 0 ? 'up' : 'down'}> {ch >= 0 ? '▲ +' : '▼ −'}{Math.abs(ch).toFixed(2)}%</span>}
          {priceStale && <span style={{ color: 'var(--warn)' }}> · price feed offline</span>}
        </span>
        {me.data && (
          <span className="top-user">
            <span>{me.data.username}@taowatch</span>
            <button type="button" className="btn" onClick={() => void signOut()}>
              sign out
            </button>
          </span>
        )}
      </div>
    </header>
  );
}
