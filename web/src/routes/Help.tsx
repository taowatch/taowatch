import { KeyBar, Page, Panel } from '../components/ui';

const KEYS: [string, string][] = [
  ['1 … 6', 'dashboard · wallets · claimable · activity · alerts · settings'],
  ['?', 'this help'],
  ['esc', 'back / close dialog'],
  ['a', 'add wallet'],
  ['/', 'focus the filter box'],
  ['r', 'next time range (pages with charts)'],
  ['t', 'chart ↔ table'],
  ['← →', 'step through chart points (chart focused)'],
  ['p', 'open positions (wallet page)'],
  ['e', 'rename wallet (wallet page)'],
  ['g / s', 'group / sort positions'],
];

const TERMS: [string, string, string?][] = [
  ['free', 'TAO in the coldkey that is not staked. Can be transferred right away.'],
  ['stake', 'TAO or alpha delegated to a validator hotkey on a subnet, always shown valued in τ.', 'stake'],
  ['τ vs α', 'τ is TAO. α is a subnet token; its τ value = amount × pool price, so it moves with the price.'],
  ['root · SN0', 'Subnet 0. Stake there is plain τ and earns root rewards, which pile up as claimable.'],
  ['claimable', "Root rewards in your validators' baskets, valued at today's alpha prices. It can go down when alpha prices fall.", 'claimable'],
  ['you receive', 'What a claim would pay now: claimable minus small "dust" holdings that are too small to sell.', 'claimable'],
  ['real Δ', 'claimable today − claimable yesterday + TAO claimed in between. Positive = rewards grew, negative = value fell.'],
  ['claim', 'Redeeming a basket: the TAO is staked onto root for you and claimable resets to about 0.'],
  ['snapshot', 'Balances read at one block. History before a wallet was added is daily for ~120 days, weekly before.'],
  ['block', 'Bittensor produces a block about every 12 seconds; the status bar shows the latest one read.'],
];

export function Help() {
  return (
    <>
      <Page title="Help">
        <div className="split">
          <Panel title="Keyboard">
            <p className="dim m0">Single letters, like htop or vim. Disabled while you type. Each key means the same thing on every page; the footer buttons do the same for mouse and touch.</p>
            <table className="tbl">
              <tbody>{KEYS.map(([k, d]) => <tr key={k}><td className="nowrap"><kbd>{k}</kbd></td><td>{d}</td></tr>)}</tbody>
            </table>
          </Panel>
          <Panel title="Glossary">
            <dl className="kv glossary">
              {TERMS.map(([t, d, tone]) => [<dt key={`t-${t}`} className={tone ?? 'strong'}>{t}</dt>, <dd key={`d-${t}`}>{d}</dd>])}
            </dl>
          </Panel>
        </div>
        <Panel title="Colors always mean the same thing">
          <p className="legend">
            <span><span className="up">▲ +</span> went up</span>
            <span><span className="down">▼ −</span> went down</span>
            <span><span className="stake">■</span> stake</span>
            <span><span>■</span> free</span>
            <span><span className="claimable">■</span> claimable only</span>
            <span><span className="warn">◐</span> connection warning</span>
            <span><span className="info">◌</span> syncing / info</span>
          </p>
        </Panel>
      </Page>
      <KeyBar keys={[{ k: 'esc', label: 'back', to: '/' }]} />
    </>
  );
}
