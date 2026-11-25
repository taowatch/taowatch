import { useEffect, useRef, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { getJson, postJson } from '../lib/api';
import { bar, block, fmt, num, RANGES, short, signed, tao, utc, type Range } from '../lib/format';
import type { ClaimRow, DayPoint, WalletDetail as Detail } from '../lib/types';
import { ChangeChart } from '../components/charts';
import { DayTable } from './Dashboard';
import { Empty, ErrorBox, KeyBar, Kpi, Loading, Page, Panel, Seg, Toast } from '../components/ui';
import { useKeys } from '../lib/hotkeys';

async function send(method: string, path: string, body?: unknown) {
  const res = await fetch(path, { method, credentials: 'same-origin', headers: body ? { 'Content-Type': 'application/json' } : undefined, body: body ? JSON.stringify(body) : undefined });
  if (!res.ok) throw new Error(((await res.json().catch(() => ({}))) as { error?: string }).error ?? res.statusText);
}

export function WalletDetail() {
  const { address = '' } = useParams();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [range, setRange] = useState<Range>('90d');
  const [view, setView] = useState<'chart' | 'table'>('chart');
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState('');
  const [confirm, setConfirm] = useState(false);
  const [typed, setTyped] = useState('');
  const [removed, setRemoved] = useState(false);
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const d = useQuery({ queryKey: ['wallet', address], queryFn: () => getJson<Detail>(`/api/wallets/${address}`), enabled: !removed });
  const h = useQuery({ queryKey: ['wallet-history', address, range], queryFn: () => getJson<DayPoint[]>(`/api/wallets/${address}/history?range=${range}`), enabled: !removed });
  const claims = useQuery({ queryKey: ['claims', address], queryFn: () => getJson<ClaimRow[]>(`/api/claims?wallet=${address}&range=all`), enabled: !removed });

  const rename = useMutation({
    mutationFn: (label: string) => send('PATCH', `/api/wallets/${address}`, { label }),
    onSuccess: () => {
      setEditing(false);
      void qc.invalidateQueries();
    },
  });
  const remove = useMutation({
    mutationFn: () => send('DELETE', `/api/wallets/${address}`),
    onSuccess: () => {
      setConfirm(false);
      setRemoved(true);
      timer.current = setTimeout(() => {
        void qc.invalidateQueries();
        navigate('/', { replace: true });
      }, 30_000);
    },
  });
  const undo = useMutation({
    mutationFn: () => postJson(`/api/wallets/${address}/restore`),
    onSuccess: () => {
      clearTimeout(timer.current);
      setRemoved(false);
      setTyped('');
      void qc.invalidateQueries();
    },
  });
  useEffect(() => () => clearTimeout(timer.current), []);

  const cycle = () => setRange(RANGES[(RANGES.indexOf(range) + 1) % RANGES.length] ?? '90d');
  const startRename = () => {
    setName(d.data?.wallet.label ?? '');
    setEditing(true);
  };
  useKeys({ r: cycle, t: () => setView(view === 'chart' ? 'table' : 'chart'), e: startRename, p: () => navigate(`/wallets/${address}/positions`), Escape: () => (confirm ? setConfirm(false) : navigate(-1)) });

  if (removed) {
    return (
      <Page title="Wallet removed">
        <Toast>
          <span>Stopped tracking {d.data?.wallet.label ?? short(address)}. Nothing happened on-chain. Undo within 30 seconds.</span>
          <button type="button" className="btn btn-accent" onClick={() => undo.mutate()} disabled={undo.isPending}>undo</button>
          <Link to="/" className="btn btn-link">dashboard</Link>
        </Toast>
      </Page>
    );
  }
  if (d.isPending) return <Page title="Wallet"><Loading /></Page>;
  if (d.isError) return <Page title="Wallet"><ErrorBox error={d.error} /><Link to="/wallets">all wallets</Link></Page>;

  const { wallet: w, positions, backfill } = d.data;
  const total = (num(w.free) ?? 0) + (num(w.stake) ?? 0) + (num(w.claimable) ?? 0);
  const ch = signed(w.change_24h);
  const stakeSum = positions.reduce((a, p) => a + (num(p.value_tao) ?? 0), 0) || 1;
  const days = h.data ?? [];

  return (
    <>
      <Page
        title={
          editing ? (
            <form onSubmit={(e) => { e.preventDefault(); rename.mutate(name.trim()); }} className="row">
              <label className="sr-only" htmlFor="rename">Wallet label</label>
              <input id="rename" autoFocus value={name} onChange={(e) => setName(e.target.value)} maxLength={24} className="inline-input" />
              <button type="submit" className="btn-primary" disabled={!name.trim()}>save</button>
              <button type="button" className="btn" onClick={() => setEditing(false)}>cancel</button>
            </form>
          ) : (
            <>{w.label} <span className="tag">read-only · coldkey</span></>
          )
        }
        sub={
          <>
            <Link to="/">dashboard</Link> / <Link to="/wallets">wallets</Link> / {w.label}
          </>
        }
        actions={
          <>
            <button type="button" className="btn" onClick={startRename}><kbd>e</kbd> rename</button>
            <Link to="/alerts" className="btn btn-link">alerts</Link>
            <button type="button" className="btn btn-danger" onClick={() => setConfirm(true)}>remove…</button>
          </>
        }
      >
        {rename.isError && <ErrorBox error={rename.error} />}
        <section className="panel addr-row">
          <code className="addr">{w.address}</code>
          <button type="button" className="btn" onClick={() => void navigator.clipboard.writeText(w.address).then(() => { setCopied(true); setTimeout(() => setCopied(false), 1500); })}>
            {copied ? 'copied ✓' : 'copy'}
          </button>
          <a className="btn btn-link" href={`https://taostats.io/account/${w.address}`} target="_blank" rel="noreferrer">explorer ↗</a>
          <span className="dim small">
            updated {block(w.block)} · history {w.history_status === 'running' || w.history_status === 'pending' ? `syncing ${w.history_progress}%${backfill ? ` (${backfill.done}/${backfill.total})` : ''}` : w.history_status}
          </span>
        </section>

        <section className="kpis" aria-label="Balances">
          <Kpi label="TOTAL" value={tao(total)} sub={<span className={ch.tone}>{ch.text} vs 24h ago</span>} />
          <Kpi label="STAKE" labelTone="stake" value={tao(w.stake)} sub={`${positions.length} position${positions.length === 1 ? '' : 's'}`} />
          <Kpi label="FREE" value={tao(w.free, 4)} sub="transferable" />
          <Kpi label="CLAIMABLE ›" labelTone="claimable" tone="claimable" to={`/claimable?wallet=${w.address}`} value={tao(w.claimable, 4)} sub={`you receive ${tao(w.redeemable, 4)}`} />
          <Kpi label="CLAIMED ALL-TIME" value={tao(w.claimed_total)} sub={w.last_claim_at ? `last ${utc(w.last_claim_at)}` : 'no claims found'} />
        </section>

        <Panel
          title="Total balance change per day"
          right={<><Seg label="View" options={['chart', 'table'] as const} value={view} onChange={setView} /><Seg label="Range" options={RANGES} value={range} onChange={setRange} /></>}
        >
          <p className="legend"><span><span className="up">█</span> up</span><span><span className="down">█</span> down</span><span>━ total (right axis)</span></p>
          {h.isPending ? <Loading /> : h.isError ? <ErrorBox error={h.error} /> : days.length < 2 ? <Empty>Not enough history yet.</Empty> : view === 'chart' ? <ChangeChart days={days} /> : <DayTable days={days} />}
          <p className="dim small m0">
            Bars compare each day with the previous point (weekly before the last ~120 days). <a href={`/api/wallets/${w.address}/history?range=${range}&format=csv`}>download CSV</a>
          </p>
        </Panel>

        <div className="split">
          <Panel title="Stake positions" right={<Link to={`/wallets/${w.address}/positions`}>all {positions.length} positions ›</Link>}>
            <div className="scroll-x">
              <table className="tbl">
                <thead><tr><th>VALIDATOR</th><th className="r hide-m">SUBNET</th><th className="r">VALUE</th><th className="hide-m">SHARE</th><th className="r">CLAIMABLE</th></tr></thead>
                <tbody>
                  {positions.slice(0, 5).map((p) => {
                    const share = ((num(p.value_tao) ?? 0) / stakeSum) * 100;
                    return (
                      <tr key={`${p.hotkey}-${p.netuid}`}>
                        <td>{p.label ?? short(p.hotkey)}</td>
                        <td className="r hide-m">SN{p.netuid}{p.netuid === 0 ? ' root' : ''}</td>
                        <td className="r stake">{tao(p.value_tao)}</td>
                        <td className="hide-m"><span aria-hidden="true" className="stake bars">{bar(share)}</span> {share.toFixed(1)}%</td>
                        <td className="r claimable">{tao(p.accrued, 4)}</td>
                      </tr>
                    );
                  })}
                  {positions.length === 0 && <tr><td colSpan={5} className="dim">No stake.</td></tr>}
                </tbody>
              </table>
            </div>
          </Panel>

          <Panel title="Claim history" right={<span className="dim">{claims.data?.length ?? 0} found</span>}>
            {claims.isPending ? <Loading /> : (claims.data ?? []).length === 0 ? <p className="dim m0">No claims found in the loaded history.</p> : (
              <ol className="list scroll-y">
                {(claims.data ?? []).map((c) => (
                  <li key={`${c.hotkey}-${c.ts}`}>
                    <span>{utc(c.ts)}</span>
                    <span className="up">+{fmt(c.amount, 4)} τ</span>
                    <span className="dim small full">{short(c.hotkey)} · {c.block ? block(c.block) : 'between snapshots'}</span>
                  </li>
                ))}
              </ol>
            )}
          </Panel>
        </div>
      </Page>

      {confirm && (
        <div className="overlay">
          <div role="dialog" aria-modal="true" aria-labelledby="rm-title" className="dialog">
            <h2 id="rm-title" className="down">Stop tracking “{w.label}”?</h2>
            <p className="m0">Its history and alerts are removed for you. Nothing happens on-chain — your TAO is not touched.</p>
            <label className="field">
              <span>Type <span className="strong">{w.label}</span> to confirm</span>
              <input autoFocus value={typed} onChange={(e) => setTyped(e.target.value)} />
            </label>
            {remove.isError && <ErrorBox error={remove.error} />}
            <div className="row">
              <button type="button" className="btn btn-danger-solid" disabled={typed.trim() !== w.label || remove.isPending} onClick={() => remove.mutate()}>Remove wallet</button>
              <button type="button" className="btn" onClick={() => setConfirm(false)}>Cancel (esc)</button>
            </div>
          </div>
        </div>
      )}

      <KeyBar keys={[{ k: 'esc', label: 'back', onClick: () => navigate(-1) }, { k: 'r', label: `range: ${range}`, onClick: cycle }, { k: 'e', label: 'rename', onClick: startRename }, { k: 'p', label: 'positions', to: `/wallets/${w.address}/positions` }, { k: '?', label: 'help', to: '/help' }]} />
    </>
  );
}
