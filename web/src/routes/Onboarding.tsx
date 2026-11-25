import { useState, type FormEvent } from 'react';
import { Link } from 'react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ApiError, getJson, postJson } from '../lib/api';
import { bar, checkAddress, short } from '../lib/format';
import type { WalletDetail, WalletRow } from '../lib/types';
import { ErrorBox, Page, Panel } from '../components/ui';

const PLANS = [
  ['rec', 'Recommended — daily for 120 days, weekly back to dTAO (Feb 2025)', '~200 archive reads · about 10–20 minutes in the background'],
  ['short', 'Last 30 days only', '~31 archive reads · a few minutes'],
  ['none', 'No history — start from now', 'charts fill in as new blocks arrive'],
] as const;

export function Onboarding() {
  const qc = useQueryClient();
  const [address, setAddress] = useState('');
  const [label, setLabel] = useState('main');
  const [plan, setPlan] = useState<'rec' | 'short' | 'none'>('rec');
  const [added, setAdded] = useState<string | null>(null);
  const check = checkAddress(address);
  const tone = !address ? 'dim' : check.ok ? 'up' : 'down';

  const add = useMutation({
    mutationFn: () => postJson<WalletRow>('/api/wallets', { address: address.trim(), label: label.trim(), history: plan }),
    onSuccess: (w) => {
      setAdded(w.address);
      void qc.invalidateQueries();
    },
  });

  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (check.ok && label.trim()) add.mutate();
  };

  return (
    <Page title="Add wallet" sub="Track any Bittensor coldkey. Read-only: TaoWatch stores only the public address and never signs or moves funds.">
      <div className="split">
        <Panel title={added ? 'Wallet added' : 'Coldkey address'}>
          {added ? (
            <Progress address={added} onAnother={() => { setAdded(null); setAddress(''); setLabel(''); add.reset(); }} />
          ) : (
            <form onSubmit={submit} className="stack">
              <label className="field">
                <span>Coldkey address (SS58)</span>
                <input value={address} onChange={(e) => setAddress(e.target.value)} spellCheck={false} autoComplete="off" placeholder="5…" aria-invalid={!!address && !check.ok} aria-describedby="addr-msg" className={address ? (check.ok ? 'ok' : 'bad') : ''} />
                <span id="addr-msg" role="status" className={`small ${tone}`}>{check.msg}</span>
              </label>
              <label className="field">
                <span>Label <span className="dim">(only you see it)</span></span>
                <input value={label} onChange={(e) => setLabel(e.target.value)} maxLength={24} required />
              </label>
              <fieldset className="field plain">
                <legend>History to load</legend>
                {PLANS.map(([id, title, sub]) => (
                  <label key={id} className="radio">
                    <input type="radio" name="plan" checked={plan === id} onChange={() => setPlan(id)} />
                    <span>
                      <span className="strong">{title}</span>
                      <br />
                      <span className="dim small">{sub}</span>
                    </span>
                  </label>
                ))}
              </fieldset>
              {add.isError && <ErrorBox error={add.error instanceof ApiError ? add.error.message : add.error} />}
              <div className="row">
                <button type="submit" className="btn-primary" disabled={!check.ok || !label.trim() || add.isPending}>
                  {add.isPending ? 'Adding…' : 'Add & start tracking'}
                </button>
              </div>
            </form>
          )}
        </Panel>
        <Panel title="What you will see">
          <ul className="bullets">
            <li><span className="stake">stake</span> across subnets and validators</li>
            <li>free balance</li>
            <li><span className="claimable">claimable</span> root rewards — and whether they went up or down each day</li>
            <li>when you last claimed, and how much you received</li>
          </ul>
          <p className="dim small m0">Current balances appear within a block (~12 s). History loads in the background from the archive node; you can leave this page.</p>
        </Panel>
      </div>
    </Page>
  );
}

function Progress({ address, onAnother }: { address: string; onAnother: () => void }) {
  const d = useQuery({ queryKey: ['wallet', address], queryFn: () => getJson<WalletDetail>(`/api/wallets/${address}`), refetchInterval: 5000 });
  const w = d.data?.wallet;
  const b = d.data?.backfill;
  const pctDone = b ? (b.total ? Math.round((b.done / b.total) * 100) : 0) : w?.history_status === 'done' || w?.history_status === 'skipped' ? 100 : 0;
  const steps: [string, string, boolean][] = [
    ['Validate address & checksum', 'server-side SS58 decode', true],
    ['Read current balances', w?.block ? `block #${w.block.toLocaleString('en-US')}` : 'waiting for the next block…', !!w?.block],
    ['Load history', b ? `${b.done} / ${b.total} snapshots · ${b.status}` : w?.history_status === 'skipped' ? 'skipped' : 'shared with another user or queued', b?.status === 'done' || w?.history_status === 'done' || w?.history_status === 'skipped'],
    ['Find claims', 'confirmed from on-chain events after history loads', b?.status === 'done' || w?.history_status === 'done'],
  ];
  return (
    <div className="stack">
      <p className="m0"><span className="up">✓ {w?.label ?? 'wallet'}</span> added · <span className="dim">{short(address)}</span></p>
      <ol className="steps">
        {steps.map(([t, sub, ok]) => (
          <li key={t}>
            <span className={ok ? 'up' : 'info'}>{ok ? '✓' : '◌'}</span>
            <span><span className="strong">{t}</span><br /><span className="dim small">{sub}</span></span>
          </li>
        ))}
      </ol>
      <div aria-hidden="true" className="info nowrap">{bar(pctDone, 25)} <span className="dim">{pctDone}%</span></div>
      {b?.last_error && <ErrorBox error={b.last_error} />}
      <div className="row">
        <Link to="/" className="btn-primary btn-link">Go to dashboard ›</Link>
        <Link to={`/wallets/${address}`} className="btn btn-link">open wallet</Link>
        <button type="button" className="btn" onClick={onAnother}>add another wallet</button>
      </div>
    </div>
  );
}
