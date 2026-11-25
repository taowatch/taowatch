import type { ReactNode } from 'react';
import { Link } from 'react-router';

export function Page({ title, sub, actions, children }: { title: ReactNode; sub?: ReactNode; actions?: ReactNode; children: ReactNode }) {
  return (
    <main className="page">
      <div className="page-head">
        <div className="stack-tight">
          <h1>{title}</h1>
          {sub && <p className="dim m0">{sub}</p>}
        </div>
        {actions && <div className="row">{actions}</div>}
      </div>
      {children}
    </main>
  );
}

export function Panel({ title, right, children, tone, id }: { title?: ReactNode; right?: ReactNode; children: ReactNode; tone?: 'claim' | 'down'; id?: string }) {
  return (
    <section className={`panel${tone ? ` panel-${tone}` : ''}`} id={id} aria-label={typeof title === 'string' ? title : undefined}>
      {(title || right) && (
        <div className="panel-head">
          {title && <h2>{title}</h2>}
          {right && <div className="row">{right}</div>}
        </div>
      )}
      {children}
    </section>
  );
}

export function Kpi({ label, value, sub, tone, to, labelTone }: { label: string; value: ReactNode; sub?: ReactNode; tone?: string; to?: string; labelTone?: string }) {
  const body = (
    <>
      <span className={labelTone ?? 'dim'}>{label}</span>
      <span className={`kpi-value ${tone ?? ''}`}>{value}</span>
      {sub && <span className="dim small">{sub}</span>}
    </>
  );
  return to ? (
    <Link to={to} className={`kpi kpi-link${labelTone === 'claimable' ? ' kpi-claim' : ''}`}>
      {body}
    </Link>
  ) : (
    <div className={`kpi${labelTone === 'claimable' ? ' kpi-claim' : ''}`}>{body}</div>
  );
}

export function Seg<T extends string>({ label, options, value, onChange }: { label: string; options: readonly (T | [T, string])[]; value: T; onChange: (v: T) => void }) {
  return (
    <div role="group" aria-label={label} className="seg seg-sm">
      {options.map((o) => {
        const [v, text] = Array.isArray(o) ? o : [o, o];
        return (
          <button key={v} type="button" aria-pressed={v === value} onClick={() => onChange(v)}>
            {text}
          </button>
        );
      })}
    </div>
  );
}

export function Loading({ what = 'Loading' }: { what?: string }) {
  return (
    <p role="status" className="dim m0">
      {what}…<span className="blink">_</span>
    </p>
  );
}

export function ErrorBox({ error }: { error: unknown }) {
  return (
    <p role="alert" className="notice notice-down">
      {error instanceof Error ? error.message : String(error)}
    </p>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <div className="empty">{children}</div>;
}

export type Key = { k: string; label: string; onClick?: () => void; to?: string };

export function KeyBar({ keys }: { keys: Key[] }) {
  return (
    <footer className="keybar">
      <div className="keybar-in">
        {keys.map((key) =>
          key.to ? (
            <Link key={key.k} to={key.to} className="key">
              <kbd>{key.k}</kbd>
              {key.label}
            </Link>
          ) : (
            <button key={key.k} type="button" className="key" onClick={key.onClick} disabled={!key.onClick}>
              <kbd>{key.k}</kbd>
              {key.label}
            </button>
          ),
        )}
      </div>
    </footer>
  );
}

export function Toast({ children, onClose }: { children: ReactNode; onClose?: () => void }) {
  return (
    <div role="status" className="toast">
      {children}
      {onClose && (
        <button type="button" className="btn" onClick={onClose} aria-label="Dismiss">
          ×
        </button>
      )}
    </div>
  );
}

/** Text that only screen readers see. */
export const SrOnly = ({ children }: { children: ReactNode }) => <span className="sr-only">{children}</span>;
