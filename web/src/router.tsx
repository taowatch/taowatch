import { createBrowserRouter, Outlet, useNavigate } from 'react-router';
import { StatusBar } from './components/StatusBar';
import { RequireAuth, useMe } from './components/RequireAuth';
import { useLive } from './lib/live';
import { useKeys } from './lib/hotkeys';
import { Login } from './routes/Login';

function Shell() {
  const navigate = useNavigate();
  const me = useMe();
  useLive(me.isSuccess);
  useKeys({
    '1': () => navigate('/'),
    '2': () => navigate('/wallets'),
    '3': () => navigate('/claimable'),
    '4': () => navigate('/activity'),
    '5': () => navigate('/alerts'),
    '6': () => navigate('/settings'),
    '?': () => navigate('/help'),
    a: () => navigate('/onboarding'),
  });
  return (
    <RequireAuth>
      <a href="#main" className="skip">Skip to content</a>
      <StatusBar />
      <div id="main">
        <Outlet />
      </div>
    </RequireAuth>
  );
}

export const router = createBrowserRouter([
  { path: '/login', element: <Login /> },
  {
    element: <Shell />,
    // Shown while a lazily loaded page is fetched on the first visit.
    hydrateFallbackElement: <p role="status" className="page dim">Loading…</p>,
    children: [
      { path: '/', lazy: () => import('./routes/Dashboard').then((m) => ({ Component: m.Dashboard })) },
      { path: '/onboarding', lazy: () => import('./routes/Onboarding').then((m) => ({ Component: m.Onboarding })) },
      { path: '/wallets', lazy: () => import('./routes/Wallets').then((m) => ({ Component: m.Wallets })) },
      { path: '/wallets/:address', lazy: () => import('./routes/WalletDetail').then((m) => ({ Component: m.WalletDetail })) },
      { path: '/wallets/:address/positions', lazy: () => import('./routes/Positions').then((m) => ({ Component: m.Positions })) },
      { path: '/claimable', lazy: () => import('./routes/Claimable').then((m) => ({ Component: m.Claimable })) },
      { path: '/activity', lazy: () => import('./routes/Activity').then((m) => ({ Component: m.Activity })) },
      { path: '/alerts', lazy: () => import('./routes/Alerts').then((m) => ({ Component: m.Alerts })) },
      { path: '/settings', lazy: () => import('./routes/Settings').then((m) => ({ Component: m.Settings })) },
      { path: '/help', lazy: () => import('./routes/Help').then((m) => ({ Component: m.Help })) },
      { path: '*', lazy: () => import('./routes/Dashboard').then((m) => ({ Component: m.Dashboard })) },
    ],
  },
]);
