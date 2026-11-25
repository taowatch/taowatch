import { useState, type FormEvent } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { getJson, postJson } from '../lib/api';
import { num, short } from '../lib/format';
import type { ClaimableResponse, DayPoint, Rule, WalletRow } from '../lib/types';
import { ErrorBox, KeyBar, Loading, Page, Panel } from '../components/ui';

const METRICS: [Rule['metric'], string, Rule['unit'][]][] = [
  ['claimable_real_delta', 'claimable real Δ (vs previous daily point)', ['pct', 'tao']],
  ['claimable_level', 'claimable level', ['tao']],
  ['days_since_claim', 'days since last claim', ['days']],
  ['total_change_24h', 'total balance change (24 h)', ['pct', 'tao']],
  ['stake_removed', 'stake removed', ['tao']],
  ['source_offline', 'data source offline or lagging', ['blocks']],
];
const metricText = (m: Rule['metric']) => METRICS.find((x) => x[0] === m)?.[1] ?? m;
const unitText = (u: Rule['unit']) => ({ pct: '%', tao: 'τ', days: 'days', blocks: 'blocks' })[u];

async function send(method: string, path: string, body?: unknown) {
  const res = await fetch(path, { method, credentials: 'same-origin', headers: body ? { 'Content-Type': 'application/json' } : undefined, body: body ? JSON.stringify(body) : undefined });
  if (!res.ok) throw new Error(((await res.json().catch(() => ({}))) as { error?: string }).error ?? res.statusText);
}

/** Days on which a claimable rule would have fired, evaluated on real history. */
function fires(r: Pick<Rule, 'metric' | 'op' | 'threshold' | 'unit'>, days: DayPoint[]): { day: string; value: string }[] {
  const t = Number(r.threshold);
  const out: { day: string; value: string }[] = [];
  days.forEach((d, i) => {
    const prev = days[i - 1];
    let v: number | null = null;
    if (r.metric === 'claimable_real_delta' && d.real_delta !== null) {
      const rd = Number(d.real_delta);
      v = r.unit === 'pct' ? (prev && (num(prev.claimable) ?? 0) > 0 ? (rd / Number(prev.claimable)) * 100 : null) : rd;
    } else if (r.metric === 'claimable_level' && d.claimable !== null) {
      // only when it first crosses
      const now = Number(d.claimable);
      const before = prev ? num(prev.claimable) : null;
      const crossed = r.op === 'above' ? now > t && (before === null || before <= t) : now < t && (before === null || before >= t);
      if (crossed) out.push({ day: d.day, value: `${now.toFixed(4)} τ` });
      return;
    }
    if (v === null) return;
    if ((r.op === 'below' && v < t) || (r.op === 'above' && v > t)) out.push({ day: d.day, value: r.unit === 'pct' ? `${v.toFixed(1)}%` : `${v.toFixed(4)} τ` });
  });
  return out;
}

export function Alerts() {
  const qc = useQueryClient();
  const rules = useQuery({ queryKey: ['alerts'], queryFn: () => getJson<Rule[]>('/api/alerts') });
  const wallets = useQuery({ queryKey: ['wallets'], queryFn: () => getJson<WalletRow[]>('/api/wallets') });
  const hist = useQuery({ queryKey: ['claimable', 'all', 'all'], queryFn: () => getJson<ClaimableResponse>('/api/claimable?wallet=all&range=all') });
  const [form, setForm] = useState<{ metric: Rule['metric']; op: Rule['op']; threshold: string; unit: Rule['unit']; wallet: string } | null>(null);

  const refresh = () => void qc.invalidateQueries({ queryKey: ['alerts'] });
  const create = useMutation({
    mutationFn: () => postJson<Rule>('/api/alerts', { metric: form!.metric, op: form!.op, threshold: form!.threshold, unit: form!.unit, wallet_address: form!.wallet === 'all' ? null : form!.wallet }),
    onSuccess: () => {
      setForm(null);
      refresh();
    },
  });
  const toggle = useMutation({ mutationFn: (r: Rule) => send('PATCH', `/api/alerts/${r.id}`, { enabled: !r.enabled }), onSuccess: refresh });
  const del = useMutation({ mutationFn: (r: Rule) => send('DELETE', `/api/alerts/${r.id}`), onSuccess: refresh });

  const days = hist.data?.days ?? [];
  const label = (a: string | null) => (a ? (wallets.data?.find((w) => w.address === a)?.label ?? short(a)) : 'all wallets');
  const supported = (m: Rule['metric']) => m === 'claimable_real_delta' || m === 'claimable_level';
  const history = (rules.data ?? [])
    .filter((r) => r.enabled && supported(r.metric))
    .flatMap((r) => fires(r, days).map((f) => ({ ...f, rule: `${metricText(r.metric)} ${r.op} ${r.threshold}${unitText(r.unit)}` })))
    .sort((a, b) => (a.day < b.day ? 1 : -1))
    .slice(0, 40);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    create.mutate();
  };

  return (
    <>
      <Page
        title="Alerts"
        actions={<button type="button" className="btn-primary" onClick={() => setForm({ metric: 'claimable_real_delta', op: 'below', threshold: '-10', unit: 'pct', wallet: 'all' })}>+ new rule</button>}
      >
        <p role="note" className="notice notice-info m0">
          Rules are saved and evaluated against your real history below. Sending notifications (Web Push, Telegram) is not built yet, so nothing is delivered.
        </p>

        {form && (
          <Panel title="New rule">
            <form onSubmit={submit} className="stack">
              <div className="row wrap-end">
                <label className="field"><span>When</span>
                  <select value={form.metric} onChange={(e) => { const m = e.target.value as Rule['metric']; setForm({ ...form, metric: m, unit: METRICS.find((x) => x[0] === m)![2][0]! }); }}>
                    {METRICS.map(([m, t]) => <option key={m} value={m}>{t}</option>)}
                  </select>
                </label>
                <label className="field"><span>is</span>
                  <select value={form.op} onChange={(e) => setForm({ ...form, op: e.target.value as Rule['op'] })}><option value="below">below</option><option value="above">above</option></select>
                </label>
                <label className="field"><span>value</span>
                  <input value={form.threshold} onChange={(e) => setForm({ ...form, threshold: e.target.value })} inputMode="decimal" className="w-num" />
                </label>
                <label className="field"><span>unit</span>
                  <select value={form.unit} onChange={(e) => setForm({ ...form, unit: e.target.value as Rule['unit'] })}>
                    {METRICS.find((x) => x[0] === form.metric)![2].map((u) => <option key={u} value={u}>{unitText(u)}</option>)}
                  </select>
                </label>
                <label className="field"><span>for wallet</span>
                  <select value={form.wallet} onChange={(e) => setForm({ ...form, wallet: e.target.value })}>
                    <option value="all">all wallets</option>
                    {(wallets.data ?? []).map((w) => <option key={w.address} value={w.address}>{w.label}</option>)}
                  </select>
                </label>
              </div>
              <p className="dim m0">
                {supported(form.metric) && !Number.isNaN(Number(form.threshold))
                  ? <>Preview: would have fired <span className="strong">{fires({ ...form, threshold: form.threshold }, days).length}</span> times since {days[0]?.day ?? '—'}.</>
                  : 'Preview is available for claimable rules.'}
              </p>
              {create.isError && <ErrorBox error={create.error} />}
              <div className="row">
                <button type="submit" className="btn-primary" disabled={create.isPending || Number.isNaN(Number(form.threshold))}>Save rule</button>
                <button type="button" className="btn" onClick={() => setForm(null)}>Cancel</button>
              </div>
            </form>
          </Panel>
        )}

        <Panel title="Rules">
          {rules.isPending ? <Loading /> : rules.isError ? <ErrorBox error={rules.error} /> : rules.data.length === 0 ? <p className="dim m0">No rules yet. Try “claimable real Δ below −10 %”.</p> : (
            <ul className="rules">
              {rules.data.map((r) => (
                <li key={r.id}>
                  <button type="button" role="switch" aria-checked={r.enabled} className={`switch${r.enabled ? ' on' : ''}`} onClick={() => toggle.mutate(r)} aria-label={`${r.enabled ? 'Disable' : 'Enable'} rule`}>
                    {r.enabled ? 'ON' : 'OFF'}
                  </button>
                  <span className="grow">
                    <span className={r.enabled ? 'strong' : 'dim'}>{metricText(r.metric)} {r.op} {r.threshold}{unitText(r.unit)}</span>
                    <br />
                    <span className="dim small">{label(r.wallet_address)} · {r.channels.join(', ')}{supported(r.metric) ? ` · ${fires(r, days).length} fires in history` : ''}</span>
                  </span>
                  <button type="button" className="btn btn-danger" onClick={() => del.mutate(r)}>delete</button>
                </li>
              ))}
            </ul>
          )}
        </Panel>

        <Panel title="History" right={<span className="dim small">enabled claimable rules, evaluated on real daily points</span>}>
          {history.length === 0 ? <p className="dim m0">Nothing would have fired.</p> : (
            <div className="scroll-x">
              <table className="tbl">
                <thead><tr><th>DAY</th><th>RULE</th><th className="r">VALUE</th></tr></thead>
                <tbody>{history.map((h, i) => <tr key={i}><td className="dim">{h.day}</td><td>{h.rule}</td><td className="r">{h.value}</td></tr>)}</tbody>
              </table>
            </div>
          )}
        </Panel>
      </Page>
      <KeyBar keys={[{ k: 'esc', label: 'back', to: '/' }, { k: '?', label: 'help', to: '/help' }]} />
    </>
  );
}
