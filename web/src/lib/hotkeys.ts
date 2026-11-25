// Single-letter shortcuts (htop/vim style). Ignored while typing or with modifier keys.
import { useEffect, useRef } from 'react';

function typing(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable;
}

/** Register `key → handler` while the component is mounted. */
export function useKeys(map: Record<string, () => void>) {
  const ref = useRef(map);
  ref.current = map;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey || typing(e.target)) return;
      if (document.querySelector('[role="dialog"][aria-modal="true"]') && e.key !== 'Escape') return;
      const fn = ref.current[e.key];
      if (fn) {
        e.preventDefault();
        fn();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}

/** Focus the page's filter box (`/`). */
export function focusFilter() {
  document.querySelector<HTMLInputElement>('input[data-filter]')?.focus();
}
