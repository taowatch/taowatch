import { useState } from 'react';
import { Link, useNavigate } from 'react-router';
import { useQuery } from '@tanstack/react-query';
import { getJson } from '../lib/api';
import { ago, block, fmt, num, RANGES, short, signed, tao, utc, type Range } from '../lib/format';
import type { ClaimRow, DayPoint, Summary, WalletRow } from '../lib/types';
import { BalanceChart } from '../components/charts';
import { Empty, ErrorBox, KeyBar, Kpi, Loading, Page, Panel, Seg } from '../components/ui';
import { useKeys } from '../lib/hotkeys';

export function Dashboard() {
  const navigate = useNavigate();
  const [range, setRange] = useState<Range>('90d');
  const [view, setView] = useState<'chart' | 'table'>('chart');
  const summary = useQuery({ queryKey: ['summary'], queryFn: () => getJson<Summary>('/api/portfolio/summary') });
  const wallets = useQuery({ queryKey: ['wallets'], queryFn: () => getJson<WalletRow[]>('/api/wallets') });
  const history = useQuery({ queryKey: ['portfolio-history', range], queryFn: () => getJson<DayPoint[]>(`/api/portfolio/history?range=${range}`) });
  const claims = useQuery({ queryKey: ['claims', 'all'], queryFn: () => getJson<ClaimRow[]>('/api/claims?range=all') });

  const cycle = () => setRange(RANGES[(RANGES.indexOf(range) + 1) % RANGES.length] ?? '90d');
  const toggle = () => setView(view === 'chart' ? 'table' : 'chart');
  useKeys({ r: cycle, t: toggle });

  if (wallets.isPending || summary.isPending) return <Page title="Portfolio"><Loading /></Page>;
  if (wallets.isError) return <Page title="Portfolio"><ErrorBox error={wallets.error} /></Page>;
  if (summary.isError) return <Page title="Portfolio"><ErrorBox error={summary.error} /></Page>;

  if (wallets.data.length === 0) {
    return (
      <Page title="Portfolio">
        <Empty>
          <p className="m0">You are not tracking any wallets yet.</p>
          <Link to="/onboarding" className="btn-primary btn-link">
            Add your first wallet
          </Link>
        </Empty>
      </Page>
    );
  }

  const s = summary.data;
  const days = history.data ?? [];
  const d24 = signed(s.change_24h);
  const c24 = signed(s.claimable_change_24h);
  const period = days.length > 1 ? Number(days[days.length - 1]!.total) - Number(days[0]!.total) : null;
  const pr = signed(period, 3);
  const dust = num(s.claimable) !== null && num(s.redeemable) !== null ? (num(s.claimable) ?? 0) - (num(s.redeemable) ?? 0) : null;
  const syncing = wallets.data.filter((w) => w.history_status === 'running' || w.history_status === 'pending');
  const labelOf = (a: string) => wallets.data.find((w) => w.address === a)?.label ?? short(a);

  return (
    <>
      <Page
        title="Portfolio"
        sub={`${s.wallets} wallet${s.wallets === 1 ? '' : 's'} · block ${block(s.block)}${syncing.length ? ` · history syncing for ${syncing.map((w) => `${w.label} ${w.history_progress}%`).join(', ')}` : ''}`}
        actions={
          <Link to="/onboarding" className="btn btn-link btn-accent">
            <kbd>a</kbd> Add wallet
          </Link>
        }
      >
        <section aria-label="Balances" className="kpis">
          <Kpi label="TOTAL" value={tao(s.total)} sub={<><span className={d24.tone}>{d24.text}</span> vs 24h ago · free + stake + claimable</>} />
          <Kpi label="STAKE" labelTone="stake" value={tao(s.stake)} />
          <Kpi label="FREE" value={tao(s.free, 4)} sub="transferable now" />
          <Kpi
            label="CLAIMABLE ›"
            labelTone="claimable"
            tone="claimable"
            to="/claimable"
            value={tao(s.claimable, 4)}
            sub={
              <>
                you receive {tao(s.redeemable, 4)} · dust {tao(dust, 4)}
                <br />
                <span className={c24.tone}>{c24.text}</span> real Δ 24h
              </>
            }
          />
          <Kpi
            label="CLAIMED ALL-TIME"
            value={tao(s.claimed_total, 3)}
            sub={s.last_claim ? <>last {utc(s.last_claim.ts)} · {block(s.last_claim.block)} · {ago(s.last_claim.ts)}</> : 'no claims found yet'}
          />
        </section>

        <Panel
          title="Balance over time"
          right={
            <>
              <Seg label="View" options={['chart', 'table'] as const} value={view} onChange={setView} />
              <Seg label="Range" options={RANGES} value={range} onChange={setRange} />
            </>
          }
        >
          <p className="legend">
            <span><span className="stake">■</span> stake</span>
            <span><span>■</span> free</span>
            <span><span className="claimable">┄</span> claimable (right axis)</span>
            <span>period <span className={pr.tone}>{pr.text}</span></span>
          </p>
          {history.isPending ? <Loading /> : history.isError ? <ErrorBox error={history.error} /> : days.length < 2 ? (
            <Empty>Not enough history yet — charts appear once there are two daily points.</Empty>
          ) : view === 'chart' ? (
            <BalanceChart days={days} />
          ) : (
            <DayTable days={days} />
          )}
          <p className="dim small m0">
            Hover, tap or focus the chart and use ← →. Points before you added a wallet come from archive snapshots. <a href={`/api/portfolio/history?range=${range}&format=csv`}>download CSV</a>
          </p>
        </Panel>

        <div className="split">
          <Panel title={`Wallets (${wallets.data.length})`} right={<Link to="/onboarding">+ add</Link>}>
            <div className="scroll-x">
              <table className="tbl">
                <thead>
                  <tr>
                    <th>WALLET</th>
                    <th className="r">FREE</th>
                    <th className="r">STAKE</th>
                    <th className="r">CLAIMABLE</th>
                    <th>LAST CLAIM</th>
                    <th className="r">TOTAL</th>
                    <th className="r">24H</th>
                  </tr>
                </thead>
                <tbody>
                  {wallets.data.map((w) => {
                    const total = (num(w.free) ?? 0) + (num(w.stake) ?? 0) + (num(w.claimable) ?? 0);
                    const ch = signed(w.change_24h);
                    return (
                      <tr key={w.address} className="click" onClick={() => navigate(`/wallets/${w.address}`)}>
                        <td>
                          <Link to={`/wallets/${w.address}`} className="strong">
                            {w.label} ›
                          </Link>
                          <div className="dim small">
                            {short(w.address)}
                            {w.history_status === 'running' || w.history_status === 'pending' ? <span className="info"> · syncing {w.history_progress}%</span> : null}
                          </div>
                        </td>
                        <td className="r">{tao(w.free, 4)}</td>
                        <td className="r stake">{tao(w.stake)}</td>
                        <td className="r claimable">{tao(w.claimable, 4)}</td>
                        <td>
                          {w.last_claim_at ? utc(w.last_claim_at).slice(0, 10) : 'never'}
                          <div className="dim small">{w.last_claim_at ? ago(w.last_claim_at) : ''}</div>
                        </td>
                        <td className="r strong">{tao(total)}</td>
                        <td className={`r ${ch.tone}`}>{ch.text}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </Panel>

          <Panel title="Recent claims" right={<Link to="/activity">all activity ›</Link>}>
            {claims.isPending ? <Loading /> : (claims.data ?? []).length === 0 ? <p className="dim m0">No claims found yet.</p> : (
              <ol className="list">
                {(claims.data ?? []).slice(0, 6).map((c) => (
                  <li key={`${c.address}-${c.hotkey}-${c.ts}`}>
                    <span><span className="claimable">CLAIM</span> {labelOf(c.address)} · {short(c.hotkey)}</span>
                    <span className="up">+{fmt(c.amount, 4)} τ</span>
                    <span className="dim small full">{utc(c.ts)} · {c.block ? block(c.block) : 'between snapshots'}</span>
                  </li>
                ))}
              </ol>
            )}
          </Panel>
        </div>
      </Page>
      <KeyBar keys={[{ k: 'a', label: 'add wallet', to: '/onboarding' }, { k: 'r', label: `range: ${range}`, onClick: cycle }, { k: 't', label: 'chart / table', onClick: toggle }, { k: '?', label: 'help', to: '/help' }]} />
    </>
  );
}

export function DayTable({ days, claimable = false }: { days: DayPoint[]; claimable?: boolean }) {
  const rows = [...days].reverse();
  return (
    <div className="scroll-box">
      <table className="tbl">
        <thead>
          <tr>
            <th>DAY</th>
            <th className="r">BLOCK</th>
            {!claimable && <th className="r">STAKE</th>}
            {!claimable && <th className="r">FREE</th>}
            <th className="r">CLAIMABLE</th>
            {claimable && <th className="r">YOU RECEIVE</th>}
            {claimable && <th className="r">REAL Δ</th>}
            <th className="r">CLAIMED</th>
            {!claimable && <th className="r">TOTAL</th>}
            {!claimable && <th className="r">CHANGE</th>}
          </tr>
        </thead>
        <tbody>
          {rows.map((d) => {
            const ch = signed(d.change);
            const rd = signed(d.real_delta);
            const claimed = num(d.claimed) ?? 0;
            return (
              <tr key={d.day}>
                <td className="dim">{d.day}{d.partial ? ' *' : ''}</td>
                <td className="r dim">{block(d.block)}</td>
                {!claimable && <td className="r stake">{tao(d.stake)}</td>}
                {!claimable && <td className="r">{tao(d.free, 4)}</td>}
                <td className="r claimable">{tao(d.claimable, 4)}</td>
                {claimable && <td className="r">{tao(d.redeemable, 4)}</td>}
                {claimable && <td className={`r ${rd.tone}`}>{rd.text}</td>}
                <td className={`r ${claimed > 0 ? 'claimable' : 'dim'}`}>{claimed > 0 ? `◆ +${fmt(claimed, 4)} τ` : '—'}</td>
                {!claimable && <td className="r strong">{tao(d.total)}</td>}
                {!claimable && <td className={`r ${ch.tone}`}>{ch.text}</td>}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
