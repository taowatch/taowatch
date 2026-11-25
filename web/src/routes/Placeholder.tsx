export function Placeholder({ title, artboard }: { title: string; artboard: string }) {
  return (
    <main className="page">
      <h1 style={{ margin: 0, fontSize: 22, color: 'var(--text-strong)' }}>{title}</h1>
      <p className="dim">
        Not built yet ({artboard}).
      </p>
    </main>
  );
}
