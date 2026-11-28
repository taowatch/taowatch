import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { getJson } from '../lib/api';
import { block, fmt, short, vname, utc } from '../lib/format';
import type { ClaimRow, EventRow, WalletRow } from '../lib/types';
import { ErrorBox, KeyBar, Loading, Page, Panel, Seg } from '../components/ui';

type Type = 'all' | 'claim' | 'stake' | 'transfer';
/** `sign`: +1 adds to the wallet, −1 takes from it, 0 neither (a move between validators). */
type Item = { key: string; ts: string; kind: string; address: string; detail: string; amount: number | null; sign: number; source: string; tone: string };

const KIND: Record<string, [string, string, number]> = {
  // label, tone, sign
  claim: ['CLAIM', 'claimable', 1],
  stake_added: ['STAKE +', 'stake', 1],
  stake_removed: ['UNSTAKE', 'stake', -1],
  stake_moved: ['MOVE', 'stake', 0], // stays staked: amount shown without a sign
  stake_transfer_in: ['STAKE IN', 'stake', 1],
  stake_transfer_out: ['STAKE OUT', 'stake', -1],
  transfer_in: ['RECEIVED', '', 1],
  transfer_out: ['SENT', '', -1],
};

export function Activity() {
  const [type, setType] = useState<Type>('all');
  const [wallet, setWallet] = useState('all');
  const [limit, setLimit] = useState(50);
  const wallets = useQuery({ queryKey: ['wallets'], queryFn: () => getJson<WalletRow[]>('/api/wallets') });
  const events = useQuery({
    queryKey: ['events', type, wallet, limit],
    queryFn: () => getJson<EventRow[]>(`/api/events?type=${type === 'claim' ? 'all' : type}&wallet=${wallet}&limit=${limit}`),
    enabled: type !== 'claim',
  });
  const claims = useQuery({ queryKey: ['claims', wallet], queryFn: () => getJson<ClaimRow[]>(`/api/claims?wallet=${wallet}&range=all`), enabled: type === 'all' || type === 'claim' });

  const label = (a: string) => wallets.data?.find((w) => w.address === a)?.label ?? short(a);
  const items: Item[] = [];
  if (type === 'all' || type === 'claim') {
    for (const c of claims.data ?? []) {
      items.push({ key: `c-${c.address}-${c.hotkey}-${c.ts}`, ts: c.ts, kind: 'claim', address: c.address, detail: `root claim · ${vname(c.hotkey, c.label)}`, amount: Number(c.amount), sign: 1, source: c.block ? block(c.block) : 'between snapshots', tone: 'claimable' });
    }
  }
  if (type !== 'claim') {
    for (const e of events.data ?? []) {
      if (e.kind === 'claim') continue; // claims come from the claims table above
      const [, tone, sign] = KIND[e.kind] ?? [e.kind, '', 1];
      const to = `${e.netuid !== null ? `SN${e.netuid}` : ''}${e.hotkey ? ` · ${vname(e.hotkey, e.label)}` : ''}`;
      const from = e.from_hotkey ? `${e.from_netuid !== null && e.from_netuid !== e.netuid ? `SN${e.from_netuid} · ` : ''}${vname(e.from_hotkey, e.from_label)} → ` : '';
      const detail = e.from_hotkey && e.from_netuid === e.netuid ? `SN${e.netuid} · ${from}${vname(e.hotkey ?? '', e.label)}` : `${from}${to}`;
      items.push({ key: `e-${e.block}-${e.idx}-${e.address}`, ts: e.ts, kind: e.kind, address: e.address, detail, amount: e.amount === null ? null : Number(e.amount), sign, source: block(e.block), tone });
    }
  }
  items.sort((a, b) => (a.ts < b.ts ? 1 : -1));
  const pending = (type !== 'claim' && events.isPending) || ((type === 'all' || type === 'claim') && claims.isPending);
  const error = events.error ?? claims.error;

  return (
    <>
      <Page
        title="Activity"
        sub="Claims are confirmed from BasketClaimed events. Stake and transfer events are recorded from the moment a wallet is tracked."
        actions={
          <>
            <label className="row dim">
              wallet
              <select value={wallet} onChange={(e) => setWallet(e.target.value)}>
                <option value="all">all wallets</option>
                {(wallets.data ?? []).map((w) => <option key={w.address} value={w.address}>{w.label}</option>)}
              </select>
            </label>
            <a className="btn btn-link" href={`/api/events?type=${type === 'claim' ? 'all' : type}&wallet=${wallet}&limit=500&format=csv`}>export csv</a>
          </>
        }
      >
        <Seg label="Event type" options={[['all', 'all'], ['claim', 'claims'], ['stake', 'stake'], ['transfer', 'transfers']] as const} value={type} onChange={setType} />
        <Panel>
          {pending ? <Loading /> : error ? <ErrorBox error={error} /> : items.length === 0 ? <p className="dim m0">No events of this type yet.</p> : (
            <div className="scroll-x">
              <table className="tbl tbl-line">
                <thead><tr><th>WHEN (UTC)</th><th>TYPE</th><th>WALLET</th><th>DETAIL</th><th className="r">AMOUNT</th><th>SOURCE</th></tr></thead>
                <tbody>
                  {items.map((i) => (
                    <tr key={i.key}>
                      <td className="dim">{utc(i.ts).replace(' UTC', '')}</td>
                      <td className={`strong ${i.tone}`}>{KIND[i.kind]?.[0] ?? i.kind}</td>
                      <td>{label(i.address)}</td>
                      <td className="line-2">{i.detail}</td>
                      <td className={`r ${i.amount === null || i.sign === 0 ? 'dim' : i.sign > 0 ? 'up' : 'down'}`}>{i.amount === null ? '' : `${i.sign > 0 ? '+' : i.sign < 0 ? '−' : ''}${fmt(i.amount, 4)} τ`}</td>
                      <td className="dim hide-m">{i.source}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {type !== 'claim' && (events.data?.length ?? 0) >= limit && (
            <button type="button" className="btn" onClick={() => setLimit(limit + 50)}>load older</button>
          )}
        </Panel>
      </Page>
      <KeyBar keys={[{ k: 'esc', label: 'back', to: '/' }, { k: '?', label: 'help', to: '/help' }]} />
    </>
  );
}
