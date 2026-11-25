import { useRef, type KeyboardEvent } from 'react';
import {
  BarController,
  BarElement,
  CategoryScale,
  Chart as ChartJS,
  Filler,
  LinearScale,
  LineController,
  LineElement,
  PointElement,
  Tooltip,
  type ChartData,
  type ChartOptions,
  type TooltipItem,
} from 'chart.js';
import { Chart } from 'react-chartjs-2';
import { num } from '../lib/format';
import type { DayPoint } from '../lib/types';

ChartJS.register(CategoryScale, LinearScale, PointElement, LineElement, BarElement, Filler, Tooltip, LineController, BarController);

const C = {
  up: '#7ee787',
  down: '#ff7b72',
  stake: '#56d4dd',
  free: '#c9d4c5',
  claim: '#f2cc60',
  text: '#e6ede3',
  dim: '#8fa093',
  grid: '#16211a',
  axis: '#64796b',
  validators: ['#79c0ff', '#d2a8ff', '#f778ba', '#a5d6ff', '#b392f0', '#7fb8c9'],
};
const font = { family: "'JetBrains Mono', monospace", size: 12 };
const tooltip = {
  backgroundColor: '#0b0f0c',
  borderColor: C.up,
  borderWidth: 1,
  titleColor: C.up,
  bodyColor: C.free,
  footerColor: C.text,
  titleFont: font,
  bodyFont: font,
  footerFont: font,
  padding: 10,
};
const reduced = typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
const base = {
  responsive: true,
  maintainAspectRatio: false,
  animation: reduced ? (false as const) : { duration: 200 },
  interaction: { mode: 'index' as const, intersect: false },
};
const xAxis = { ticks: { color: C.dim, font, maxTicksLimit: 8, maxRotation: 0 }, grid: { color: C.grid } };
const sgn = (v: number | null, d = 4) => (v === null ? 'n/a' : `${v >= 0 ? '▲ +' : '▼ −'}${Math.abs(v).toFixed(d)} τ`);

/** Focusable wrapper: ← / → step through points and show the tooltip. */
function ChartFrame({ label, height, children }: { label: string; height: number; children: (ref: React.RefObject<ChartJS | null>) => React.ReactNode }) {
  const ref = useRef<ChartJS | null>(null);
  const idx = useRef<number | null>(null);
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    const ch = ref.current;
    const n = ch?.data.labels?.length ?? 0;
    if (!ch || !n) return;
    e.preventDefault();
    const cur = idx.current ?? n;
    const i = Math.max(0, Math.min(n - 1, cur + (e.key === 'ArrowLeft' ? -1 : 1)));
    idx.current = i;
    const active = ch.data.datasets.map((_, d) => ({ datasetIndex: d, index: i })).filter((a) => ch.data.datasets[a.datasetIndex]?.data[i] != null);
    const el = active[0] && ch.getDatasetMeta(active[0].datasetIndex).data[i];
    ch.setActiveElements(active);
    if (el) ch.tooltip?.setActiveElements(active, { x: el.x, y: el.y });
    ch.update('none');
  };
  return (
    <div className="chart" style={{ height }} tabIndex={0} role="img" aria-label={`${label} Use ← and → to step through points; switch to table view for exact values.`} onKeyDown={onKey}>
      {children(ref)}
    </div>
  );
}

export function BalanceChart({ days }: { days: DayPoint[] }) {
  const data: ChartData<'line', (number | null)[], string> = {
    labels: days.map((d) => d.day),
    datasets: [
      { label: 'Stake', data: days.map((d) => num(d.stake)), borderColor: C.stake, backgroundColor: 'rgba(86,212,221,.16)', fill: 'origin', stack: 'bal', yAxisID: 'y', pointRadius: 0, pointHoverRadius: 4, borderWidth: 2 },
      { label: 'Free', data: days.map((d) => num(d.free)), borderColor: C.free, backgroundColor: 'rgba(201,212,197,.18)', fill: '-1', stack: 'bal', yAxisID: 'y', pointRadius: 0, pointHoverRadius: 4, borderWidth: 1 },
      { label: 'Claimable', data: days.map((d) => num(d.claimable)), borderColor: C.claim, borderDash: [5, 4], yAxisID: 'y1', pointRadius: 0, pointHoverRadius: 4, borderWidth: 2, spanGaps: false },
    ],
  };
  const options: ChartOptions<'line'> = {
    ...base,
    plugins: {
      tooltip: {
        ...tooltip,
        callbacks: {
          label: (c: TooltipItem<'line'>) => ` ${c.dataset.label?.padEnd(10)}${c.raw == null ? 'n/a' : `${(c.raw as number).toFixed(c.dataset.label === 'Claimable' ? 4 : 3)} τ`}`,
          footer: (items) => {
            const d = days[items[0]?.dataIndex ?? 0];
            return d ? `Total     ${Number(d.total).toFixed(3)} τ\nBlock     #${d.block.toLocaleString('en-US')}${d.partial ? '\n(some wallets have no data this day)' : ''}` : '';
          },
        },
      },
    },
    scales: {
      x: xAxis,
      y: { stacked: true, position: 'left', ticks: { color: C.dim, font, callback: (v) => `${Number(v).toLocaleString('en-US')} τ` }, grid: { color: C.grid } },
      y1: { position: 'right', beginAtZero: true, ticks: { color: C.claim, font, callback: (v) => `${Number(v).toFixed(1)} τ` }, grid: { drawOnChartArea: false } },
    },
  };
  return (
    <ChartFrame label={`Balance chart, ${days.length} days, stake and free stacked, claimable on the right axis.`} height={320}>
      {(ref) => <Chart ref={ref as never} type="line" data={data} options={options} />}
    </ChartFrame>
  );
}

export function ClaimableChart({ days }: { days: DayPoint[] }) {
  const dr = days.map((d) => num(d.real_delta));
  const data: ChartData<'bar' | 'line', (number | null)[], string> = {
    labels: days.map((d) => d.day),
    datasets: [
      { type: 'bar', label: 'Real Δ', data: dr, yAxisID: 'y', backgroundColor: dr.map((v) => (v === null ? '#2b3b30' : v >= 0 ? C.up : C.down)), borderWidth: 0 },
      {
        type: 'line',
        label: 'Claimable',
        data: days.map((d) => num(d.claimable)),
        yAxisID: 'y1',
        borderColor: C.claim,
        borderWidth: 2,
        pointRadius: days.map((d) => ((num(d.claimed) ?? 0) > 0 ? 6 : 0)),
        pointStyle: 'rectRot',
        pointBackgroundColor: C.claim,
        pointHoverRadius: 5,
      },
    ],
  };
  const options: ChartOptions<'bar' | 'line'> = {
    ...base,
    plugins: {
      tooltip: {
        ...tooltip,
        callbacks: {
          label: (c) => (c.dataset.type === 'bar' ? ` Real Δ     ${sgn(c.raw as number | null)}` : ` Claimable  ${(c.raw as number).toFixed(4)} τ`),
          footer: (items) => {
            const d = days[items[0]?.dataIndex ?? 0];
            if (!d) return '';
            const claimed = num(d.claimed) ?? 0;
            return `Receive    ${d.redeemable ? Number(d.redeemable).toFixed(4) : '—'} τ${claimed > 0 ? `\nClaimed    +${claimed.toFixed(4)} τ ◆` : ''}\nBlock      #${d.block.toLocaleString('en-US')}`;
          },
        },
      },
    },
    scales: {
      x: xAxis,
      y: {
        position: 'left',
        title: { display: true, text: 'real Δ / day (τ)', color: C.dim, font },
        ticks: { color: C.dim, font, callback: (v) => `${Number(v) > 0 ? '+' : ''}${Number(v).toFixed(2)}` },
        grid: { color: (c) => (c.tick.value === 0 ? C.axis : C.grid) },
      },
      y1: { position: 'right', beginAtZero: true, title: { display: true, text: 'claimable (τ)', color: C.claim, font }, ticks: { color: C.claim, font }, grid: { drawOnChartArea: false } },
    },
  };
  return (
    <ChartFrame label={`Claimable level and real daily change, ${days.length} days.`} height={340}>
      {(ref) => <Chart ref={ref as never} type="bar" data={data as ChartData<'bar'>} options={options as ChartOptions<'bar'>} />}
    </ChartFrame>
  );
}

export function ValidatorChart({ labels, series }: { labels: string[]; series: { name: string; values: (number | null)[] }[] }) {
  const data: ChartData<'line', (number | null)[], string> = {
    labels,
    datasets: series.map((s, i) => {
      const color = C.validators[i % C.validators.length] ?? C.stake;
      return { label: s.name, data: s.values, borderColor: color, backgroundColor: `${color}33`, fill: i === 0 ? 'origin' : '-1', stack: 'v', pointRadius: 0, pointHoverRadius: 4, borderWidth: 1.5 };
    }),
  };
  const options: ChartOptions<'line'> = {
    ...base,
    plugins: {
      tooltip: {
        ...tooltip,
        callbacks: {
          label: (c) => ` ${c.dataset.label}  ${((c.raw as number | null) ?? 0).toFixed(4)} τ`,
          footer: (items) => `Total      ${items.reduce((a, c) => a + ((c.raw as number | null) ?? 0), 0).toFixed(4)} τ`,
        },
      },
    },
    scales: { x: xAxis, y: { stacked: true, beginAtZero: true, ticks: { color: C.dim, font, callback: (v) => `${Number(v).toFixed(1)} τ` }, grid: { color: C.grid } } },
  };
  return (
    <ChartFrame label="Claimable per validator, stacked." height={240}>
      {(ref) => <Chart ref={ref as never} type="line" data={data} options={options} />}
    </ChartFrame>
  );
}

export function ChangeChart({ days }: { days: DayPoint[] }) {
  const ch = days.map((d) => num(d.change));
  const data: ChartData<'bar' | 'line', (number | null)[], string> = {
    labels: days.map((d) => d.day),
    datasets: [
      { type: 'bar', label: 'Change', data: ch, yAxisID: 'y', backgroundColor: ch.map((v) => (v === null ? '#2b3b30' : v >= 0 ? C.up : C.down)) },
      { type: 'line', label: 'Total', data: days.map((d) => num(d.total)), yAxisID: 'y1', borderColor: C.text, borderWidth: 1.5, pointRadius: 0, pointHoverRadius: 4 },
    ],
  };
  const options: ChartOptions<'bar' | 'line'> = {
    ...base,
    plugins: {
      tooltip: {
        ...tooltip,
        callbacks: {
          label: (c) => (c.dataset.type === 'bar' ? ` Change  ${sgn(c.raw as number | null)}` : ` Total   ${(c.raw as number).toFixed(3)} τ`),
          footer: (items) => {
            const d = days[items[0]?.dataIndex ?? 0];
            return d ? `stake ${Number(d.stake).toFixed(3)} · free ${Number(d.free).toFixed(4)}${d.claimable ? ` · claimable ${Number(d.claimable).toFixed(4)}` : ''}\n#${d.block.toLocaleString('en-US')}` : '';
          },
        },
      },
    },
    scales: {
      x: xAxis,
      y: { ticks: { color: C.dim, font }, grid: { color: (c) => (c.tick.value === 0 ? C.axis : C.grid) }, title: { display: true, text: 'change (τ)', color: C.dim, font } },
      y1: { position: 'right', ticks: { color: C.text, font }, grid: { drawOnChartArea: false }, title: { display: true, text: 'total (τ)', color: C.text, font } },
    },
  };
  return (
    <ChartFrame label={`Total balance change per day, ${days.length} days.`} height={280}>
      {(ref) => <Chart ref={ref as never} type="bar" data={data as ChartData<'bar'>} options={options as ChartOptions<'bar'>} />}
    </ChartFrame>
  );
}
