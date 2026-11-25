import { useState } from 'react';
import { Link, useNavigate } from 'react-router';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { startRegistration, type PublicKeyCredentialCreationOptionsJSON } from '@simplewebauthn/browser';
import { getJson, postJson, type ChainStatus } from '../lib/api';
import { ago, block, short, utc } from '../lib/format';
import { authConfig, deviceName, explain, logout, setPassword } from '../lib/passkey';
import type { Invite, PasskeyInfo, UserInfo, WalletRow } from '../lib/types';
import { useMe } from '../components/RequireAuth';
import { ErrorBox, KeyBar, Loading, Page, Panel } from '../components/ui';

async function send(method: string, path: string, body?: unknown) {
  const res = await fetch(path, { method, credentials: 'same-origin', headers: body ? { 'Content-Type': 'application/json' } : undefined, body: body ? JSON.stringify(body) : undefined });
  if (!res.ok) throw new Error(((await res.json().catch(() => ({}))) as { error?: string }).error ?? res.statusText);
}

export function Settings() {
  const me = useMe();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const passkeys = useQuery({ queryKey: ['passkeys'], queryFn: () => getJson<PasskeyInfo[]>('/api/passkeys') });
  const methods = useQuery({ queryKey: ['methods'], queryFn: () => getJson<{ password: boolean; passkeys: number }>('/api/auth/methods') });
  const [pw, setPw] = useState({ current: '', next: '', repeat: '' });
  const [pwMsg, setPwMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const codes = useQuery({ queryKey: ['codes'], queryFn: () => getJson<{ unused: number; total: number }>('/api/recovery-codes') });
  const wallets = useQuery({ queryKey: ['wallets'], queryFn: () => getJson<WalletRow[]>('/api/wallets') });
  const chain = useQuery({ queryKey: ['chain-status'], queryFn: () => getJson<ChainStatus>('/api/chain/status') });
  const admin = me.data?.role === 'admin';
  const invites = useQuery({ queryKey: ['invites'], queryFn: () => getJson<Invite[]>('/api/admin/invites'), enabled: admin });
  const users = useQuery({ queryKey: ['users'], queryFn: () => getJson<UserInfo[]>('/api/admin/users'), enabled: admin });
  const cfg = useQuery({ queryKey: ['auth-config'], queryFn: authConfig, enabled: admin });

  const [confirmPk, setConfirmPk] = useState<PasskeyInfo | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const [newCodes, setNewCodes] = useState<string[] | null>(null);
  const [error, setError] = useState('');

  const inv = (k: string) => void qc.invalidateQueries({ queryKey: [k] });
  const addPasskey = useMutation({
    mutationFn: async () => {
      const ccr = await postJson<{ publicKey: PublicKeyCredentialCreationOptionsJSON }>('/api/passkeys/register/start');
      const credential = await startRegistration({ optionsJSON: ccr.publicKey });
      await postJson('/api/passkeys/register/finish', { credential, name: deviceName() });
    },
    onSuccess: () => { setError(''); inv('passkeys'); },
    onError: (e) => setError(explain(e)),
  });
  const removePk = useMutation({ mutationFn: (id: string) => send('DELETE', `/api/passkeys/${id}`), onSuccess: () => { setConfirmPk(null); inv('passkeys'); }, onError: (e) => setError(explain(e)) });
  const renamePk = useMutation({ mutationFn: (r: { id: string; name: string }) => send('PATCH', `/api/passkeys/${r.id}`, { name: r.name }), onSuccess: () => { setRenaming(null); inv('passkeys'); } });
  const regen = useMutation({ mutationFn: () => postJson<string[]>('/api/recovery-codes/regenerate'), onSuccess: (c) => { setNewCodes(c); inv('codes'); } });
  const newInvite = useMutation({ mutationFn: () => postJson<Invite>('/api/admin/invites'), onSuccess: () => inv('invites') });
  const revoke = useMutation({ mutationFn: (code: string) => send('DELETE', `/api/admin/invites/${code}`), onSuccess: () => inv('invites') });
  const signOut = async () => { await logout(); qc.clear(); navigate('/login', { replace: true }); };

  const onlyOne = (passkeys.data?.length ?? 0) <= 1 && !methods.data?.password;
  const savePassword = useMutation({
    mutationFn: () => setPassword(pw.next, pw.current),
    onSuccess: () => {
      setPw({ current: '', next: '', repeat: '' });
      setPwMsg({ ok: true, text: 'Password saved.' });
      inv('methods');
    },
    onError: (e) => setPwMsg({ ok: false, text: explain(e) }),
  });

  return (
    <>
      <Page title="Settings" sub={me.data ? `signed in as ${me.data.username} · ${me.data.role}` : ''}>
        <nav aria-label="Settings sections" className="row small">
          <a href="#passkeys">passkeys</a><a href="#password">password</a><a href="#wallets">wallets</a><a href="#notify">notifications</a><a href="#source">data source</a>{admin && <a href="#admin">users & invites</a>}<a href="#session">session</a>
        </nav>
        {error && <ErrorBox error={error} />}

        <Panel id="passkeys" title="Passkeys" right={<button type="button" className="btn-primary" onClick={() => addPasskey.mutate()} disabled={addPasskey.isPending}>+ add passkey on this device</button>}>
          {passkeys.isPending ? <Loading /> : (
            <ul className="rules">
              {(passkeys.data ?? []).map((p) => (
                <li key={p.id}>
                  <span className="grow">
                    {renaming?.id === p.id ? (
                      <form className="row" onSubmit={(e) => { e.preventDefault(); renamePk.mutate(renaming); }}>
                        <input autoFocus value={renaming.name} onChange={(e) => setRenaming({ ...renaming, name: e.target.value })} className="inline-input" aria-label="Passkey name" />
                        <button type="submit" className="btn-primary">save</button>
                        <button type="button" className="btn" onClick={() => setRenaming(null)}>cancel</button>
                      </form>
                    ) : (
                      <>
                        <span className="strong">{p.name}</span><br />
                        <span className="dim small">added {utc(p.created_at).slice(0, 10)} · last used {p.last_used_at ? ago(p.last_used_at) : 'never'}</span>
                      </>
                    )}
                  </span>
                  <button type="button" className="btn" onClick={() => setRenaming({ id: p.id, name: p.name })}>rename</button>
                  <button type="button" className="btn btn-danger" disabled={onlyOne} title={onlyOne ? 'Set a password or add another passkey before removing your last one' : undefined} onClick={() => setConfirmPk(p)}>remove</button>
                </li>
              ))}
            </ul>
          )}
          {confirmPk && (
            <div role="alertdialog" aria-labelledby="pk-c" className="notice notice-down">
              <span id="pk-c">Remove <span className="strong">{confirmPk.name}</span>? That device will no longer be able to sign in.</span>
              <span className="row">
                <button type="button" className="btn btn-danger-solid" onClick={() => removePk.mutate(confirmPk.id)}>Remove</button>
                <button type="button" className="btn" onClick={() => setConfirmPk(null)}>Keep</button>
              </span>
            </div>
          )}
          <div className="row between top-line">
            <span>Recovery codes: <span className="strong">{codes.data ? `${codes.data.unused} of ${codes.data.total} unused` : '…'}</span></span>
            <button type="button" className="btn" onClick={() => regen.mutate()} disabled={regen.isPending}>regenerate…</button>
          </div>
          {newCodes && (
            <div className="notice notice-up">
              <strong>New recovery codes — the old ones no longer work. Save these now.</strong>
              <pre className="codes">{newCodes.join('\n')}</pre>
              <span className="row">
                <button type="button" className="btn" onClick={() => void navigator.clipboard.writeText(newCodes.join('\n'))}>copy</button>
                <button type="button" className="btn-primary" onClick={() => setNewCodes(null)}>done</button>
              </span>
            </div>
          )}
        </Panel>

        <Panel id="password" title="Password" right={<span className="dim small">{methods.data?.password ? 'username + password sign-in is on' : 'not set — passkey only'}</span>}>
          <form
            className="row wrap-end"
            onSubmit={(e) => {
              e.preventDefault();
              if (pw.next !== pw.repeat) return setPwMsg({ ok: false, text: 'The two new passwords do not match.' });
              savePassword.mutate();
            }}
          >
            <input type="text" name="username" autoComplete="username" value={me.data?.username ?? ''} readOnly hidden />
            {methods.data?.password && (
              <label className="field"><span>Current password</span>
                <input type="password" autoComplete="current-password" value={pw.current} onChange={(e) => setPw({ ...pw, current: e.target.value })} required />
              </label>
            )}
            <label className="field"><span>New password <span className="dim">(≥ 10)</span></span>
              <input type="password" autoComplete="new-password" minLength={10} maxLength={128} value={pw.next} onChange={(e) => setPw({ ...pw, next: e.target.value })} required />
            </label>
            <label className="field"><span>Repeat</span>
              <input type="password" autoComplete="new-password" minLength={10} maxLength={128} value={pw.repeat} onChange={(e) => setPw({ ...pw, repeat: e.target.value })} required />
            </label>
            <button type="submit" className="btn-primary" disabled={savePassword.isPending}>{methods.data?.password ? 'Change password' : 'Set password'}</button>
          </form>
          {pwMsg && <p role={pwMsg.ok ? 'status' : 'alert'} className={`m0 ${pwMsg.ok ? 'up' : 'down'}`}>{pwMsg.text}</p>}
        </Panel>

        <Panel id="wallets" title="Wallets" right={<Link to="/onboarding" className="btn btn-link">+ add wallet</Link>}>
          {(wallets.data ?? []).length === 0 ? <p className="dim m0">No wallets.</p> : (
            <ul className="rules">
              {(wallets.data ?? []).map((w) => (
                <li key={w.address}>
                  <span className="grow"><span className="strong">{w.label}</span><br /><span className="dim small break">{w.address} · history {w.history_status}</span></span>
                  <Link to={`/wallets/${w.address}`} className="btn btn-link">open · rename · remove</Link>
                </li>
              ))}
            </ul>
          )}
        </Panel>

        <Panel id="notify" title="Notification channels">
          <p className="dim m0">Web Push and Telegram delivery are not built yet. Alert rules can already be created and previewed on the Alerts page.</p>
        </Panel>

        <Panel id="source" title="Data source">
          <dl className="kv">
            <dt>live block</dt><dd>{block(chain.data?.live_block)} · seen {chain.data?.seen_at ? ago(chain.data.seen_at) : 'never'}</dd>
            <dt>lag</dt><dd>{chain.data?.lag_blocks ?? '—'} blocks</dd>
            <dt>archive</dt><dd>{chain.data?.archive_ok === null || chain.data?.archive_ok === undefined ? 'not used yet' : chain.data.archive_ok ? 'reachable' : 'unreachable — history backfill paused'}</dd>
            <dt>snapshots</dt><dd>hourly + daily at 00:00 UTC</dd>
          </dl>
        </Panel>

        {admin && (
          <Panel id="admin" title="Users & invites" right={<button type="button" className="btn-primary" onClick={() => newInvite.mutate()}>+ create invite</button>}>
            {cfg.data && cfg.data.registration !== 'invite' && (
              <p className="notice m0">
                {cfg.data.registration === 'open'
                  ? 'Sign-up is open (TAOWATCH_REGISTRATION=open): anyone who can reach this site can create an account; invite codes are not needed.'
                  : 'Sign-up is closed (TAOWATCH_REGISTRATION=closed): invite codes are not accepted. Set it to invite to let invited people sign up.'}
              </p>
            )}
            <div className="scroll-x">
              <table className="tbl tbl-line">
                <thead><tr><th>INVITE</th><th>STATUS</th><th>EXPIRES</th><th /></tr></thead>
                <tbody>
                  {(invites.data ?? []).map((i) => {
                    const status = i.used_by ? `used by ${i.used_by}` : i.revoked_at ? 'revoked' : Date.parse(i.expires_at) < Date.now() ? 'expired' : 'unused';
                    return (
                      <tr key={i.code}>
                        <td className="claimable strong">{i.code}</td>
                        <td className={status === 'unused' ? 'up' : 'dim'}>{status}</td>
                        <td className="dim">{utc(i.expires_at).slice(0, 10)}</td>
                        <td className="r">{status === 'unused' && <button type="button" className="btn" onClick={() => revoke.mutate(i.code)}>revoke</button>}</td>
                      </tr>
                    );
                  })}
                  {(invites.data ?? []).length === 0 && <tr><td colSpan={4} className="dim">No invites yet.</td></tr>}
                </tbody>
              </table>
            </div>
            <div className="scroll-x">
              <table className="tbl">
                <thead><tr><th>USER</th><th>ROLE</th><th className="r">WALLETS</th><th>SINCE</th></tr></thead>
                <tbody>
                  {(users.data ?? []).map((u) => (
                    <tr key={u.username}><td className="strong">{u.username}{u.username === me.data?.username ? ' (you)' : ''}</td><td>{u.role}</td><td className="r">{u.wallets}</td><td className="dim">{utc(u.created_at).slice(0, 10)}</td></tr>
                  ))}
                </tbody>
              </table>
            </div>
          </Panel>
        )}

        <Panel id="session" title="Session" right={<button type="button" className="btn" onClick={() => void signOut()}>sign out</button>}>
          <p className="dim m0">Sessions expire after 30 days without use. {me.data ? `User id ${short(me.data.id)}.` : ''}</p>
        </Panel>
      </Page>
      <KeyBar keys={[{ k: 'esc', label: 'back', to: '/' }, { k: '?', label: 'help', to: '/help' }]} />
    </>
  );
}
