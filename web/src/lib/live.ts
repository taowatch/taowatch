// Realtime: one WebSocket per tab. Indexer notices invalidate the matching queries.
import { useEffect, useRef } from 'react';
import { useQueryClient } from '@tanstack/react-query';

type Msg =
  | { type: 'hello' | 'block'; block: number | null }
  | { type: 'wallet'; address: string }
  | { type: 'event'; address: string; block: number }
  | { type: 'backfill'; address: string; progress: string }
  | { type: 'price'; usd: string; change_24h: string | null }
  | { type: 'resync' };

export function useLive(enabled: boolean) {
  const qc = useQueryClient();
  const last = useRef(0);
  useEffect(() => {
    if (!enabled) return;
    let ws: WebSocket | null = null;
    let stop = false;
    let attempt = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    // Balances change every block; refresh at most every 10 s.
    const refreshBalances = () => {
      const now = Date.now();
      if (now - last.current < 10_000) return;
      last.current = now;
      for (const k of ['summary', 'wallets', 'wallet', 'portfolio-history', 'claimable']) void qc.invalidateQueries({ queryKey: [k] });
    };

    const connect = () => {
      const url = `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws`;
      ws = new WebSocket(url);
      ws.onopen = () => {
        attempt = 0;
      };
      ws.onmessage = (e) => {
        let m: Msg;
        try {
          m = JSON.parse(String(e.data)) as Msg;
        } catch {
          return;
        }
        switch (m.type) {
          case 'hello':
          case 'block':
            void qc.invalidateQueries({ queryKey: ['chain-status'] });
            break;
          case 'wallet':
            refreshBalances();
            break;
          case 'event':
            for (const k of ['events', 'claims']) void qc.invalidateQueries({ queryKey: [k] });
            // Stake moved / claimed: the wallet notice for this same block follows right after the
            // event, so let it through the 10 s throttle instead of showing old positions for a while.
            last.current = 0;
            break;
          case 'price':
            // The price arrives with the message: update the cache directly, no refetch.
            qc.setQueryData(['price'], { usd: m.usd, change_24h: m.change_24h, ts: new Date().toISOString() });
            break;
          case 'backfill':
          case 'resync':
            void qc.invalidateQueries();
            break;
        }
      };
      ws.onclose = () => {
        if (stop) return;
        attempt += 1;
        timer = setTimeout(connect, Math.min(30_000, 1000 * 2 ** attempt));
      };
    };
    connect();
    return () => {
      stop = true;
      clearTimeout(timer);
      ws?.close();
    };
  }, [enabled, qc]);
}
