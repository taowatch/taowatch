// Passkey ceremonies against the API (webauthn-rs on the server, @simplewebauthn/browser here).
import {
  browserSupportsWebAuthn,
  browserSupportsWebAuthnAutofill,
  startAuthentication,
  startRegistration,
  type PublicKeyCredentialCreationOptionsJSON,
  type PublicKeyCredentialRequestOptionsJSON,
} from '@simplewebauthn/browser';
import { ApiError, getJson, postJson, type Me } from './api';

// webauthn-rs wraps the options: { publicKey: {...} }
type Creation = { publicKey: PublicKeyCredentialCreationOptionsJSON };
type Request = { publicKey: PublicKeyCredentialRequestOptionsJSON };

export { browserSupportsWebAuthn };

/** A readable default name for the new passkey, e.g. "Chrome on macOS". */
export function deviceName(): string {
  const ua = navigator.userAgent;
  const browser = /Edg\//.test(ua) ? 'Edge' : /Firefox\//.test(ua) ? 'Firefox' : /Chrome\//.test(ua) ? 'Chrome' : /Safari\//.test(ua) ? 'Safari' : 'Browser';
  const os = /iPhone|iPad/.test(ua) ? 'iOS' : /Android/.test(ua) ? 'Android' : /Mac OS X/.test(ua) ? 'macOS' : /Windows/.test(ua) ? 'Windows' : /Linux/.test(ua) ? 'Linux' : 'device';
  return `${browser} on ${os}`;
}

/** Turn browser/WebAuthn/API errors into one sentence for the user. */
export function explain(e: unknown): string {
  if (e instanceof ApiError) return e.message;
  if (e instanceof Error) {
    if (e.name === 'NotAllowedError') return 'The passkey prompt was closed or timed out. Nothing was changed.';
    if (e.name === 'InvalidStateError') return 'This device already has a passkey for this account.';
    if (e.name === 'SecurityError') return 'Passkeys need HTTPS and a matching domain (RP_ID). Open the site by its configured name.';
    if (e.name === 'AbortError') return 'The passkey request was cancelled.';
    return e.message;
  }
  return String(e);
}

export async function register(username: string, invite: string | undefined, displayName?: string): Promise<{ user: Me; recovery_codes: string[] }> {
  const ccr = await postJson<Creation>('/api/auth/register/start', { username, invite: invite || undefined, display_name: displayName || undefined });
  const credential = await startRegistration({ optionsJSON: ccr.publicKey });
  return postJson('/api/auth/register/finish', { credential, name: deviceName() });
}

/** Username empty ⇒ discoverable login (the browser lists saved passkeys). */
export async function login(username?: string): Promise<Me> {
  const rcr = await postJson<Request>('/api/auth/login/start', { username: username || undefined });
  const credential = await startAuthentication({ optionsJSON: rcr.publicKey });
  return postJson('/api/auth/login/finish', credential);
}

/** Conditional UI: offer saved passkeys in the username field's autofill. Resolves only if the user picks one. */
export async function loginWithAutofill(): Promise<Me | null> {
  if (!(await browserSupportsWebAuthnAutofill())) return null;
  const rcr = await postJson<Request>('/api/auth/login/start', {});
  const credential = await startAuthentication({ optionsJSON: rcr.publicKey, useBrowserAutofill: true });
  return postJson('/api/auth/login/finish', credential);
}

/** The recovery code was accepted (and used up) and the user is signed in, but the follow-up step failed. */
export class RecoveryIncomplete extends Error {
  constructor(readonly reason: unknown) {
    super(explain(reason));
  }
}

/** Recovery code → signed in → either set a new password, or create a passkey on this device. */
export async function recover(username: string, code: string, newPassword?: string): Promise<Me> {
  const me = await postJson<Me>('/api/auth/recover', { username, code });
  try {
    if (newPassword) {
      await postJson('/api/auth/password', { new: newPassword });
      return me;
    }
    const ccr = await postJson<Creation>('/api/passkeys/register/start');
    const credential = await startRegistration({ optionsJSON: ccr.publicKey });
    await postJson('/api/passkeys/register/finish', { credential, name: deviceName() });
    return me;
  } catch (e) {
    throw new RecoveryIncomplete(e);
  }
}

export const logout = () => postJson('/api/auth/logout');

// ── username + password ──────────────────────────────────────────────────────

export type AuthConfig = { registration: 'open' | 'invite' | 'closed'; first_user: boolean };
export const authConfig = () => getJson<AuthConfig>('/api/auth/config');

export const passwordLogin = (username: string, password: string) => postJson<Me>('/api/auth/password/login', { username, password });

export const passwordRegister = (username: string, password: string, invite?: string) =>
  postJson<{ user: Me; recovery_codes: string[] }>('/api/auth/password/register', { username, password, invite: invite || undefined });

export const setPassword = (newPassword: string, current?: string) => postJson('/api/auth/password', { new: newPassword, current: current || undefined });
