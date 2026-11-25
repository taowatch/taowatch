import { useMemo, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router';
import { useQuery } from '@tanstack/react-query';
import { getJson } from '../lib/api';
import { bar, block, num, short, tao, utc } from '../lib/format';
import type { PositionRow, WalletDetail } from '../lib/types';
import { ErrorBox, KeyBar, Kpi, Loading, Page, Panel, Seg } from '../components/ui';
import { focusFilter, useKeys } from '../lib/hotkeys';

const NET_COLORS = ['#56d4dd', '#79c0ff', '#d2a8ff', '#f778ba', '#a5d6ff', '#b392f0', '#7fb8c9', '#c9d4c5'];
type Group = 'subnet' | 'validator' | 'none';
type Sort = 'value' | 'claimable';

export function Positions() {
  const { address = '' } = useParams();
  const navigate = useNavigate();
  const d = useQuery({ queryKey: ['wallet', address], queryFn: () => getJson<WalletDetail>(`/api/wallets/${address}`) });
  const [group, setGroup] = useState<Group>('subnet');
  const [sort, setSort] = useState<Sort>('value');
  const [hideDust, setHideDust] = useState(true);
  const [q, setQ] = useState('');
  const [net, setNet] = useState<number | null>(null);
  const [closed, setClosed] = useState<Record<string, boolean>>({});
  const [sel, setSel] = useState<string | null>(null);

  const GROUPS: Group[] = ['subnet', 'validator', 'none'];
  const SORTS: Sort[] = ['value', 'claimable'];
  useKeys({
    g: () => setGroup(GROUPS[(GROUPS.indexOf(group) + 1) % GROUPS.length] ?? 'subnet'),
    s: () => setSort(SORTS[(SORTS.indexOf(sort) + 1) % SORTS.length] ?? 'value'),
    '/': focusFilter,
    Escape: () => navigate(`/wallets/${address}`),
  });

  const positions = d.data?.positions ?? [];
  const total = positions.reduce((a, p) => a + (num(p.value_tao) ?? 0), 0) || 1;
  const nets = useMemo(() => [...new Set(positions.map((p) => p.netuid))].sort((a, b) => a - b), [positions]);
  const color = (n: number) => NET_COLORS[nets.indexOf(n) % NET_COLORS.length] ?? NET_COLORS[0]!;
  const key = (p: PositionRow) => `${p.hotkey}-${p.netuid}`;
  const vname = (p: PositionRow) => p.label ?? short(p.hotkey);

  let list = positions.filter((p) => !(hideDust && (num(p.value_tao) ?? 0) < 0.05));
  if (net !== null) list = list.filter((p) => p.netuid === net);
  if (q.trim()) {
    const s = q.trim().toLowerCase();
    list = list.filter((p) => `sn${p.netuid} ${p.hotkey} ${p.label ?? ''}`.toLowerCase().includes(s));
  }
  const by = (p: PositionRow) => (sort === 'value' ? num(p.value_tao) ?? 0 : num(p.accrued) ?? 0);
  const groups = new Map<string, PositionRow[]>();
  for (const p of list) {
    const g = group === 'subnet' ? `SN${p.netuid}` : group === 'validator' ? vname(p) : 'all positions';
    groups.set(g, [...(groups.get(g) ?? []), p]);
  }
  const sorted = [...groups.entries()]
    .map(([g, ps]) => ({ g, ps: [...ps].sort((a, b) => by(b) - by(a)), v: ps.reduce((a, p) => a + (num(p.value_tao) ?? 0), 0) }))
    .sort((a, b) => b.v - a.v);
  const subnetTotals = nets.map((n) => ({ n, v: positions.filter((p) => p.netuid === n).reduce((a, p) => a + (num(p.value_tao) ?? 0), 0) })).sort((a, b) => b.v - a.v);
  const selected = positions.find((p) => key(p) === sel) ?? list[0];
  const claimSum = positions.reduce((a, p) => a + (num(p.accrued) ?? 0), 0);
  const top3 = [...positions].sort((a, b) => (num(b.value_tao) ?? 0) - (num(a.value_tao) ?? 0)).slice(0, 3).reduce((a, p) => a + (num(p.value_tao) ?? 0), 0);

  if (d.isPending) return <Page title="Stake positions"><Loading /></Page>;
  if (d.isError) return <Page title="Stake positions"><ErrorBox error={d.error} /></Page>;
  const w = d.data.wallet;

  return (
    <>
      <Page title="Stake positions" sub={<><Link to="/">dashboard</Link> / <Link to={`/wallets/${address}`}>{w.label}</Link> / positions</>}>
        <section className="kpis" aria-label="Summary">
          <Kpi label="TOTAL STAKE" labelTone="stake" value={tao(w.stake)} />
          <Kpi label="POSITIONS" value={String(positions.length)} sub={`${nets.length} subnet${nets.length === 1 ? '' : 's'} · ${new Set(positions.map((p) => p.hotkey)).size} validators`} />
          <Kpi label="CLAIMABLE (ROOT)" labelTone="claimable" tone="claimable" value={tao(claimSum, 4)} sub={`receive ${tao(w.redeemable, 4)}`} />
          <Kpi label="CONCENTRATION" value={`${((top3 / total) * 100).toFixed(1)}%`} sub="in the top 3 positions" />
        </section>

        <Panel title="Allocation by subnet" right={<span className="dim small">click a tile to filter</span>}>
          <div className="tiles">
            {subnetTotals.map(({ n, v }) => {
              const p = (v / total) * 100;
              return (
                <button key={n} type="button" className="tile" aria-pressed={net === n} onClick={() => setNet(net === n ? null : n)} style={{ flexGrow: Math.max(1, Math.round(p * 10)), borderColor: color(n), borderWidth: net === n ? 2 : 1 }}>
                  <span className="card-head"><span style={{ color: color(n) }} className="strong">SN{n}{n === 0 ? ' root' : ''}</span><span className="dim">{p.toFixed(1)}%</span></span>
                  <span>{tao(v, 2)}</span>
                </button>
              );
            })}
          </div>
        </Panel>

        <div className="toolbar">
          <div className="row">
            <span className="dim">group</span>
            <Seg label="Group by" options={['subnet', 'validator', 'none'] as const} value={group} onChange={(g) => { setGroup(g); setClosed({}); }} />
            <span className="dim">sort</span>
            <Seg label="Sort by" options={[['value', 'value'], ['claimable', 'claimable']] as const} value={sort} onChange={setSort} />
          </div>
          <div className="row">
            <label className="check"><input type="checkbox" checked={hideDust} onChange={() => setHideDust(!hideDust)} /> hide dust &lt; 0.05 τ</label>
            <label className="filter">
              <span aria-hidden="true">/</span>
              <span className="sr-only">Filter positions</span>
              <input data-filter type="search" value={q} onChange={(e) => setQ(e.target.value)} placeholder="sn0, hotkey, validator…" />
            </label>
          </div>
        </div>

        <div className="split">
          <Panel title={`${list.length} position${list.length === 1 ? '' : 's'}${positions.length - list.length ? ` (${positions.length - list.length} hidden by filters)` : ''}`}>
            <div className="scroll-x">
              <table className="tbl">
                <thead><tr><th>{group === 'subnet' ? 'VALIDATOR' : group === 'validator' ? 'SUBNET' : 'POSITION'}</th><th className="r">AMOUNT</th><th className="r">VALUE</th><th>SHARE</th><th className="r">CLAIMABLE</th></tr></thead>
                <tbody>
                  {sorted.map(({ g, ps, v }) => {
                    const open = !closed[g];
                    const share = (v / total) * 100;
                    return [
                      <tr key={`g-${g}`} className="group-row">
                        <td colSpan={2}>
                          <button type="button" className="plain-btn" aria-expanded={open} onClick={() => setClosed({ ...closed, [g]: open })}>
                            <span className="dim">{open ? '▾' : '▸'}</span> <span className="strong">{g}</span> <span className="dim">{ps.length} pos</span>
                          </button>
                        </td>
                        <td className="r strong">{tao(v)}</td>
                        <td><span aria-hidden="true" className="bars">{bar(share)}</span> {share.toFixed(1)}%</td>
                        <td className="r claimable">{tao(ps.reduce((a, p) => a + (num(p.accrued) ?? 0), 0) || null, 4)}</td>
                      </tr>,
                      ...(open
                        ? ps.map((p) => {
                            const s = ((num(p.value_tao) ?? 0) / total) * 100;
                            return (
                              <tr key={key(p)} className={selected && key(selected) === key(p) ? 'selected' : ''}>
                                <td className="indent">
                                  <button type="button" className="plain-btn" onClick={() => setSel(key(p))}>
                                    {group === 'validator' ? `SN${p.netuid}` : vname(p)} <span className="dim">{group === 'validator' ? '' : short(p.hotkey)}</span>
                                  </button>
                                </td>
                                <td className="r">{p.netuid === 0 ? tao(p.amount) : `${Number(p.amount).toFixed(2)} α`}</td>
                                <td className="r">{tao(p.value_tao)}</td>
                                <td><span aria-hidden="true" className="bars" style={{ color: color(p.netuid) }}>{bar(s)}</span> {s < 0.01 ? '<0.01' : s.toFixed(2)}%</td>
                                <td className="r claimable">{tao(p.accrued, 4)}</td>
                              </tr>
                            );
                          })
                        : []),
                    ];
                  })}
                </tbody>
              </table>
            </div>
          </Panel>

          {selected && (
            <Panel title={`SN${selected.netuid} · ${vname(selected)}`}>
              <dl className="kv">
                <dt>hotkey</dt><dd className="break">{selected.hotkey}</dd>
                <dt>subnet</dt><dd>SN{selected.netuid}{selected.netuid === 0 ? ' (root)' : ''}</dd>
                <dt>amount</dt><dd>{selected.netuid === 0 ? tao(selected.amount, 4) : `${Number(selected.amount).toFixed(4)} α`}</dd>
                <dt>price</dt><dd>{selected.price ? `${Number(selected.price).toFixed(6)} τ/α` : '1 τ (root)'}</dd>
                <dt>value</dt><dd className="strong">{tao(selected.value_tao, 4)}</dd>
                <dt>claimable</dt><dd className="claimable">{selected.accrued ? `${tao(selected.accrued, 4)} (receive ${tao(selected.redeemable, 4)})` : '— (only root earns claimable)'}</dd>
                <dt>claimed</dt><dd>{tao(selected.claimed_total, 4)}</dd>
                <dt>last claim</dt><dd>{selected.last_claim_at ? `${utc(selected.last_claim_at)} · ${block(selected.last_claim_block)}` : 'never'}</dd>
              </dl>
              <p className="dim small m0">Read-only. TaoWatch never signs or unstakes for you.</p>
            </Panel>
          )}
        </div>
      </Page>
      <KeyBar keys={[{ k: 'esc', label: 'back', to: `/wallets/${address}` }, { k: 'g', label: `group: ${group}`, onClick: () => setGroup(GROUPS[(GROUPS.indexOf(group) + 1) % 3] ?? 'subnet') }, { k: 's', label: `sort: ${sort}`, onClick: () => setSort(sort === 'value' ? 'claimable' : 'value') }, { k: '/', label: 'filter', onClick: focusFilter }, { k: '?', label: 'help', to: '/help' }]} />
    </>
  );
}
