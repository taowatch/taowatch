import { useQuery } from '@tanstack/react-query';
import { NavLink } from 'react-router';
import { getJson, netState, type ChainStatus } from '../lib/api';

const TABS = [
  ['/', '1 dashboard'],
  ['/wallets', '2 wallets'],
  ['/claimable', '3 claimable'],
  ['/activity', '4 activity'],
  ['/alerts', '5 alerts'],
  ['/settings', '6 settings'],
] as const;

const NET = {
  live: { icon: '●', color: 'var(--up)', label: (b: string) => `LIVE #${b}` },
  lag: { icon: '◐', color: 'var(--warn)', label: (b: string) => `LAG · last #${b}` },
  offline: { icon: '○', color: 'var(--down)', label: (b: string) => `OFFLINE · last #${b}` },
  syncing: { icon: '◌', color: 'var(--info)', label: () => 'SYNCING history' },
} as const;

export function StatusBar() {
  const { data } = useQuery({ queryKey: ['chain-status'], queryFn: () => getJson<ChainStatus>('/api/chain/status'), refetchInterval: 12_000 });
  const state = netState(data);
  const block = data?.live_block?.toLocaleString('en-US') ?? '—';
  const net = NET[state];
  return (
    <header style={{ background: 'var(--bg-raised)', borderBottom: '1px solid var(--line)' }}>
      <div style={{ maxWidth: 1400, margin: '0 auto', padding: '6px 20px', display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: '6px 18px' }}>
        <NavLink to="/" style={{ color: 'var(--accent)', fontWeight: 700, textDecoration: 'none' }}>[taowatch]</NavLink>
        <nav aria-label="Main" style={{ display: 'flex', flexWrap: 'wrap', gap: 2 }}>
          {TABS.map(([to, label]) => (
            <NavLink
              key={to}
              to={to}
              end={to === '/'}
              style={({ isActive }) => ({
                display: 'inline-flex', alignItems: 'center', minHeight: 32, padding: '0 10px', textDecoration: 'none',
                background: isActive ? 'var(--accent)' : 'transparent', color: isActive ? 'var(--bg)' : 'var(--text)', fontWeight: isActive ? 700 : 400,
              })}
            >
              {label}
            </NavLink>
          ))}
        </nav>
        <span role="status" style={{ marginLeft: 'auto', display: 'inline-flex', gap: 8, alignItems: 'center', minHeight: 28, padding: '0 10px', border: `1px solid ${net.color}`, color: net.color }}>
          <span aria-hidden="true">{net.icon}</span>
          {net.label(block)}
        </span>
      </div>
    </header>
  );
}
