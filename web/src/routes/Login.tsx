import { useEffect, useState, type FormEvent } from 'react';
import { useNavigate } from 'react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { authConfig, browserSupportsWebAuthn, explain, login, loginWithAutofill, passwordLogin, passwordRegister, recover, RecoveryIncomplete, register } from '../lib/passkey';

type Mode = 'signin' | 'register';
type Method = 'password' | 'passkey';
type Step = 'form' | 'waiting' | 'error' | 'codes' | 'recover';

const LOGO = ` ╔╦╗┌─┐┌─┐  ╦ ╦┌─┐┌┬┐┌─┐┬ ┬
  ║ ├─┤│ │  ║║║├─┤ │ │  ├─┤
  ╩ ┴ ┴└─┘  ╚╩╝┴ ┴ ┴ └─┘┴ ┴  τ`;

function savedMethod(): Method {
  try {
    return localStorage.getItem('tw.method') === 'passkey' ? 'passkey' : 'password';
  } catch {
    return 'password';
  }
}

export function Login() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const cfg = useQuery({ queryKey: ['auth-config'], queryFn: authConfig });
  const [mode, setMode] = useState<Mode>('signin');
  const [method, setMethodState] = useState<Method>(savedMethod);
  const [step, setStep] = useState<Step>('form');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [invite, setInvite] = useState('');
  const [code, setCode] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [error, setError] = useState('');
  const [codes, setCodes] = useState<string[]>([]);
  const [recovering, setRecovering] = useState(false);
  const [recovered, setRecovered] = useState(false);
  const webauthn = browserSupportsWebAuthn();

  const firstUser = cfg.data?.first_user ?? false;
  const needInvite = cfg.data?.registration === 'invite' && !firstUser;
  const signupClosed = cfg.data?.registration === 'closed' && !firstUser;

  const setMethod = (m: Method) => {
    setMethodState(m);
    setError('');
    try {
      localStorage.setItem('tw.method', m);
    } catch {
      /* private mode */
    }
  };

  const done = async () => {
    await qc.invalidateQueries({ queryKey: ['me'] });
    navigate('/', { replace: true });
  };

  // Offer saved passkeys in the username field's autofill (conditional UI).
  useEffect(() => {
    if (!webauthn || mode !== 'signin' || method !== 'passkey') return;
    let live = true;
    loginWithAutofill()
      .then((me) => {
        if (live && me) void done();
      })
      .catch(() => {
        /* aborted when another ceremony starts */
      });
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, method, webauthn]);

  const run = async (fn: () => Promise<unknown>, waiting: boolean) => {
    setError('');
    if (waiting) setStep('waiting');
    try {
      await fn();
    } catch (e) {
      setError(explain(e));
      setStep(waiting ? 'error' : 'form');
    }
  };

  const onSignin = (e: FormEvent) => {
    e.preventDefault();
    if (method === 'password') {
      void run(async () => {
        await passwordLogin(username.trim(), password);
        await done();
      }, false);
    } else {
      void run(async () => {
        await login(username.trim());
        await done();
      }, true);
    }
  };

  const onRegister = (e: FormEvent) => {
    e.preventDefault();
    if (method === 'password') {
      if (password !== confirm) {
        setError('The two passwords do not match.');
        return;
      }
      void run(async () => {
        const r = await passwordRegister(username.trim(), password, invite.trim());
        setCodes(r.recovery_codes);
        setStep('codes');
      }, false);
    } else {
      void run(async () => {
        const r = await register(username.trim(), invite.trim());
        setCodes(r.recovery_codes);
        setStep('codes');
      }, true);
    }
  };

  // Stays on the recovery form on failure. If the code was accepted but setting the password or
  // passkey failed, the code is used up and the user is signed in: say so and offer Settings.
  const onRecover = async (e: FormEvent) => {
    e.preventDefault();
    setError('');
    setRecovering(true);
    try {
      await recover(username.trim(), code.trim(), newPassword || undefined);
      await done();
    } catch (err) {
      if (err instanceof RecoveryIncomplete) {
        setRecovered(true);
        setError(`Your recovery code was accepted and you are signed in (the code is now used), but ${newPassword ? 'setting the password' : 'creating the passkey'} failed: ${err.message} Finish it in Settings.`);
      } else {
        setError(explain(err));
      }
    } finally {
      setRecovering(false);
    }
  };

  const pickMode = (m: Mode) => {
    setMode(m);
    setStep('form');
    setError('');
  };

  const passkeyBlocked = method === 'passkey' && !webauthn;

  return (
    <div className="login-wrap">
      <div className="login-box">
        <pre role="img" aria-label="TaoWatch" className="logo">{LOGO}</pre>
        <p className="dim m0">Read-only tracker for Bittensor wallets. You will never be asked for a seed phrase or private key.</p>

        {(step === 'form' || step === 'waiting') && (
          <>
            <div role="tablist" aria-label="Mode" className="seg">
              <button type="button" role="tab" aria-selected={mode === 'signin'} onClick={() => pickMode('signin')}>
                Sign in
              </button>
              <button type="button" role="tab" aria-selected={mode === 'register'} onClick={() => pickMode('register')}>
                Create account
              </button>
            </div>
            <div className="row between">
              <span className="dim">{mode === 'signin' ? 'Sign in with' : 'Sign up with'}</span>
              <div role="group" aria-label="Method" className="seg seg-sm">
                <button type="button" aria-pressed={method === 'password'} onClick={() => setMethod('password')}>
                  username + password
                </button>
                <button type="button" aria-pressed={method === 'passkey'} onClick={() => setMethod('passkey')}>
                  passkey
                </button>
              </div>
            </div>
          </>
        )}

        {passkeyBlocked && step === 'form' && (
          <p role="alert" className="notice notice-down">
            {window.isSecureContext
              ? 'This browser does not support passkeys. Use username + password, or a recent Chrome, Safari, Edge or Firefox.'
              : `Passkeys need a secure page: open this site over HTTPS, or as http://localhost:${location.port || '80'} when testing locally. Username + password works here.`}
          </p>
        )}

        {error && step === 'form' && (
          <p role="alert" className="notice notice-down">
            {error}
          </p>
        )}

        {step === 'form' && mode === 'signin' && (
          <form onSubmit={onSignin} className="stack">
            <label className="field">
              <span>
                Username{method === 'passkey' && <span className="dim"> (optional — leave empty to pick a saved passkey)</span>}
              </span>
              <input
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                autoComplete={method === 'passkey' ? 'username webauthn' : 'username'}
                autoCapitalize="none"
                spellCheck={false}
                required={method === 'password'}
              />
            </label>
            {method === 'password' && (
              <label className="field">
                <span>Password</span>
                <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" required />
              </label>
            )}
            <button type="submit" className="btn-primary" disabled={passkeyBlocked}>
              {method === 'password' ? 'Sign in' : 'Sign in with passkey'}
            </button>
            <button type="button" className="link" onClick={() => { setError(''); setStep('recover'); }}>
              Forgot your password or lost your device? Use a recovery code
            </button>
          </form>
        )}

        {step === 'form' && mode === 'register' && signupClosed && (
          <p className="notice">Sign-up is closed on this server. New accounts can only be created after the admin re-opens sign-up.</p>
        )}

        {step === 'form' && mode === 'register' && !signupClosed && (
          <form onSubmit={onRegister} className="stack">
            {firstUser && <p className="notice notice-up m0">This is the first account on this server — it will be the admin.</p>}
            {needInvite && (
              <label className="field">
                <span>Invite code</span>
                <input value={invite} onChange={(e) => setInvite(e.target.value)} placeholder="XXXX-XXXX" autoCapitalize="characters" spellCheck={false} required />
                <span className="dim small">Ask the admin of this TaoWatch server.</span>
              </label>
            )}
            <label className="field">
              <span>Username</span>
              <input value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="username" autoCapitalize="none" spellCheck={false} required minLength={2} maxLength={32} pattern="[a-z0-9_.\-]+" title="2–32 characters: a–z 0–9 _ . -" />
            </label>
            {method === 'password' && (
              <>
                <label className="field">
                  <span>Password <span className="dim">(at least 10 characters)</span></span>
                  <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" required minLength={10} maxLength={128} />
                </label>
                <label className="field">
                  <span>Repeat password</span>
                  <input type="password" value={confirm} onChange={(e) => setConfirm(e.target.value)} autoComplete="new-password" required minLength={10} maxLength={128} />
                </label>
              </>
            )}
            <button type="submit" className="btn-primary" disabled={passkeyBlocked}>
              {method === 'password' ? 'Create account' : 'Create passkey'}
            </button>
            {method === 'password' && <span className="dim small">You can add a passkey later in Settings.</span>}
          </form>
        )}

        {step === 'waiting' && (
          <div role="status" className="notice notice-info">
            <span>&gt; waiting for your passkey…</span>
            <span className="dim">Confirm with Face ID, Touch ID, Windows Hello, your phone, or a security key.</span>
          </div>
        )}

        {step === 'error' && (
          <div role="alert" className="notice notice-down">
            <strong>That did not work</strong>
            <span>{error}</span>
            <span className="row">
              <button type="button" className="btn-primary" onClick={() => setStep('form')}>
                Try again
              </button>
              <button type="button" className="btn" onClick={() => { setMethod('password'); setStep('form'); }}>
                Use password instead
              </button>
            </span>
          </div>
        )}

        {step === 'codes' && (
          <div className="notice notice-up">
            <strong>Account created. Save your recovery codes.</strong>
            <span>Each code works once if you forget your password or lose your devices. They will not be shown again.</span>
            <pre className="codes">{codes.join('\n')}</pre>
            <span className="row">
              <button type="button" className="btn" onClick={() => void navigator.clipboard.writeText(codes.join('\n'))}>
                copy
              </button>
              <button type="button" className="btn-primary" onClick={() => void done()}>
                I saved them — continue ›
              </button>
            </span>
          </div>
        )}

        {step === 'recover' && (
          <form onSubmit={(e) => void onRecover(e)} className="stack">
            <strong>Recover your account</strong>
            <label className="field">
              <span>Username</span>
              <input value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="username" autoCapitalize="none" required />
            </label>
            <label className="field">
              <span>Recovery code</span>
              <input value={code} onChange={(e) => setCode(e.target.value)} placeholder="XXXX-XXXX-XXXX" autoCapitalize="characters" required />
            </label>
            <label className="field">
              <span>New password <span className="dim">(leave empty to create a passkey on this device instead)</span></span>
              <input type="password" value={newPassword} onChange={(e) => setNewPassword(e.target.value)} autoComplete="new-password" minLength={10} maxLength={128} />
            </label>
            {recovering && !newPassword && <span role="status" className="dim">&gt; waiting for your passkey…</span>}
            {error && <span role="alert" className="down">{error}</span>}
            {recovered ? (
              <span className="row">
                <button type="button" className="btn-primary" onClick={() => void qc.invalidateQueries({ queryKey: ['me'] }).then(() => navigate('/settings', { replace: true }))}>
                  Continue to Settings ›
                </button>
              </span>
            ) : (
              <span className="row">
                <button type="submit" className="btn-primary" disabled={recovering || (!newPassword && !webauthn)}>
                  {newPassword ? 'Verify and set password' : 'Verify and create passkey'}
                </button>
                <button type="button" className="btn" onClick={() => pickMode('signin')}>
                  Back to sign in
                </button>
              </span>
            )}
          </form>
        )}
      </div>
    </div>
  );
}
