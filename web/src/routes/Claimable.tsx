import { useState } from 'react';
import { Link, useSearchParams } from 'react-router';
import { useQuery } from '@tanstack/react-query';
import { getJson } from '../lib/api';
import { block, fmt, num, pct, short, signed, tao, utc } from '../lib/format';
import type { ClaimableResponse, ClaimRow, Summary, WalletRow } from '../lib/types';
import { ClaimableChart, ValidatorChart } from '../components/charts';
import { DayTable } from './Dashboard';
import { Empty, ErrorBox, KeyBar, Kpi, Loading, Page, Panel, Seg } from '../components/ui';
import { useKeys } from '../lib/hotkeys';

type R = '7d' | '30d' | 'all';
const RS: R[] = ['7d', '30d', 'all'];

export function Claimable() {
  const [params, setParams] = useSearchParams();
  const wallet = params.get('wallet') ?? 'all';
  const [range, setRange] = useState<R>('all');
  const [view, setView] = useState<'chart' | 'table'>('chart');
  const wallets = useQuery({ queryKey: ['wallets'], queryFn: () => getJson<WalletRow[]>('/api/wallets') });
  const summary = useQuery({ queryKey: ['summary'], queryFn: () => getJson<Summary>('/api/portfolio/summary') });
  const c = useQuery({ queryKey: ['claimable', wallet, range], queryFn: () => getJson<ClaimableResponse>(`/api/claimable?wallet=${wallet}&range=${range}`) });
  const claims = useQuery({ queryKey: ['claims', wallet], queryFn: () => getJson<ClaimRow[]>(`/api/claims?wallet=${wallet}&range=all`) });

  const cycle = () => setRange(RS[(RS.indexOf(range) + 1) % RS.length] ?? 'all');
  const toggle = () => setView(view === 'chart' ? 'table' : 'chart');
  useKeys({ r: cycle, t: toggle });

  const w = wallet === 'all' ? null : wallets.data?.find((x) => x.address === wallet);
  const now = w ? { cl: w.claimable, rd: w.redeemable } : { cl: summary.data?.claimable ?? null, rd: summary.data?.redeemable ?? null };
  const days = c.data?.days ?? [];
  const withD = days.filter((d) => d.real_delta !== null);
  const last = days[days.length - 1];
  const prev = days[days.length - 2];
  const d1 = num(last?.real_delta);
  const d1pct = d1 !== null && prev && (num(prev.claimable) ?? 0) > 0 ? (d1 / (num(prev.claimable) ?? 1)) * 100 : null;
  const last7 = withD.slice(-7).reduce((a, d) => a + (num(d.real_delta) ?? 0), 0);
  const avg = withD.length ? withD.reduce((a, d) => a + (num(d.real_delta) ?? 0), 0) / withD.length : null;
  const up = withD.filter((d) => (num(d.real_delta) ?? 0) > 0).length;
  const down = withD.filter((d) => (num(d.real_delta) ?? 0) < 0).length;
  const worst = withD.reduce<(typeof withD)[number] | null>((m, d) => (m === null || (num(d.real_delta) ?? 0) < (num(m.real_delta) ?? 0) ? d : m), null);
  const first = days[0]?.day;
  const inRange = (claims.data ?? []).filter((x) => !first || x.ts.slice(0, 10) >= first);
  const claimedInRange = inRange.reduce((a, x) => a + (num(x.amount) ?? 0), 0);
  const s1 = signed(d1);
  const s7 = signed(last7);
  const sAvg = signed(avg);
  const sW = signed(num(worst?.real_delta));

  // Per-validator stacked series aligned to the day axis.
  const labels = days.map((d) => d.day);
  const series = (c.data?.validators ?? []).map((v) => {
    const m = new Map(v.points);
    return { name: v.label ?? short(v.hotkey), label: v.label, hotkey: v.hotkey, values: labels.map((l) => num(m.get(l) ?? null)) };
  });

  return (
    <>
      <Page
        title="Claimable — is it going up or down?"
        sub={days.length ? `${days.length} daily points from ${first} · basket data only exists since the root-claim basket launched` : 'loading…'}
        actions={
          <>
            <label className="row dim">
              wallet
              <select value={wallet} onChange={(e) => setParams(e.target.value === 'all' ? {} : { wallet: e.target.value })}>
                <option value="all">all wallets</option>
                {(wallets.data ?? []).map((x) => <option key={x.address} value={x.address}>{x.label} · {short(x.address)}</option>)}
              </select>
            </label>
            <Seg label="Range" options={[['7d', '7d'], ['30d', '30d'], ['all', 'since basket']] as const} value={range} onChange={setRange} />
          </>
        }
      >
        <section className="kpis" aria-label="Claimable summary">
          <Kpi label="CLAIMABLE NOW" labelTone="claimable" tone="claimable" value={tao(now.cl, 4)} sub={<>you receive {tao(now.rd, 4)} · dust {tao(num(now.cl) !== null && num(now.rd) !== null ? (num(now.cl) ?? 0) - (num(now.rd) ?? 0) : null, 4)}</>} />
          <Kpi label="LAST DAY (REAL Δ)" value={<span className={s1.tone}>{s1.text}</span>} sub={`${pct(d1pct)}${prev ? ` · since ${prev.day}` : ''}`} />
          <Kpi label="LAST 7 POINTS" value={<span className={s7.tone}>{s7.text}</span>} sub={<>avg <span className={sAvg.tone}>{sAvg.text}</span> / day in range</>} />
          <Kpi label="UP / DOWN DAYS" value={<><span className="up">▲ {up}</span> <span className="dim">/</span> <span className="down">▼ {down}</span></>} sub={worst ? <>worst <span className={sW.tone}>{sW.text}</span> on {worst.day}</> : '—'} />
          <Kpi label="CLAIMED IN RANGE" value={tao(claimedInRange, 4)} sub={`${inRange.length} claim${inRange.length === 1 ? '' : 's'}${inRange[0] ? ` · last ${utc(inRange[0].ts).slice(0, 10)}` : ''}`} />
        </section>

        <Panel title="Daily change and level" right={<Seg label="View" options={['chart', 'table'] as const} value={view} onChange={setView} />}>
          <p className="legend">
            <span><span className="up">█</span> went up</span>
            <span><span className="down">█</span> went down</span>
            <span><span className="claimable">━</span> claimable (right axis)</span>
            <span><span className="claimable">◆</span> claim — level resets</span>
          </p>
          {c.isPending ? <Loading /> : c.isError ? <ErrorBox error={c.error} /> : days.length < 2 ? (
            <Empty>No claimable history yet. It starts at the basket launch, or when history finishes loading.</Empty>
          ) : view === 'chart' ? <ClaimableChart days={days} /> : <DayTable days={days} claimable />}
          <details className="explain">
            <summary>How is “real Δ” calculated, and why can claimable go down?</summary>
            <p><span className="strong">Real Δ = claimable today − claimable yesterday + TAO claimed in between.</span> Adding the claim back means a claim day is not counted as a drop.</p>
            <p>Claimable is what you would receive if you redeemed your root baskets now. A basket holds alpha from many subnets valued at current pool prices, so claimable falls when those prices fall even if nothing was taken out.</p>
            <p className="dim">Source: BetaBasketRuntimeApi (get_root_basket_owed, get_root_basket_claim_previews); claims from BasketClaimed events. <a href={`/api/claimable?wallet=${wallet}&range=${range}&format=csv`}>download CSV</a></p>
          </details>
        </Panel>

        <Panel title="Which validator is it coming from?">
          {series.length === 0 ? <p className="dim m0">No per-validator data yet.</p> : (
            <>
              <ValidatorChart labels={labels} series={series} />
              <div className="scroll-x">
                <table className="tbl">
                  <thead><tr><th>VALIDATOR</th><th className="r">CLAIMABLE</th><th className="r">7-POINT REAL Δ</th><th className="r">CLAIMED</th><th>LAST CLAIM</th></tr></thead>
                  <tbody>
                    {series.map((s) => {
                      const vals = s.values.filter((v): v is number => v !== null);
                      const lastV = vals[vals.length - 1] ?? null;
                      const idx7 = Math.max(0, s.values.length - 8);
                      const startV = s.values.slice(idx7).find((v) => v !== null) ?? null;
                      const since = labels[idx7] ?? '';
                      const mine = (claims.data ?? []).filter((x) => x.hotkey === s.hotkey);
                      const claimed7 = mine.filter((x) => x.ts.slice(0, 10) > since).reduce((a, x) => a + (num(x.amount) ?? 0), 0);
                      const d7 = signed(lastV !== null && startV !== null ? lastV - startV + claimed7 : null);
                      return (
                        <tr key={s.hotkey}>
                          <td>{s.label ? <>{s.label} <span className="dim">{short(s.hotkey)}</span></> : short(s.hotkey)}</td>
                          <td className="r claimable">{tao(lastV, 4)}</td>
                          <td className={`r ${d7.tone}`}>{d7.text}</td>
                          <td className="r">{tao(mine.reduce((a, x) => a + (num(x.amount) ?? 0), 0), 4)}</td>
                          <td>{mine[0] ? <>{utc(mine[0].ts)} <span className="dim">+{fmt(mine[0].amount, 4)} τ · {block(mine[0].block)}</span></> : 'never'}</td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            </>
          )}
        </Panel>

        <section className="panel row between">
          <span>Get notified when claimable drops more than 10 % in a day or goes above a level.</span>
          <Link to="/alerts" className="btn btn-link">set up alerts ›</Link>
        </section>
      </Page>
      <KeyBar keys={[{ k: 'r', label: `range: ${range}`, onClick: cycle }, { k: 't', label: 'chart / table', onClick: toggle }, { k: 'esc', label: 'back', to: '/' }, { k: '?', label: 'help', to: '/help' }]} />
    </>
  );
}
