import { createBrowserRouter, Outlet } from 'react-router';
import { StatusBar } from './components/StatusBar';
import { Placeholder } from './routes/Placeholder';
import { Dashboard } from './routes/Dashboard';

function Shell() {
  return (
    <>
      <StatusBar />
      <Outlet />
    </>
  );
}

// Routes mirror the design artboards (PLAN.md §7).
export const router = createBrowserRouter([
  { path: '/login', element: <Placeholder title="Sign in" artboard="Login" /> },
  {
    element: <Shell />,
    children: [
      { path: '/', element: <Dashboard /> },
      { path: '/onboarding', element: <Placeholder title="Add wallet" artboard="Onboarding" /> },
      { path: '/wallets', element: <Placeholder title="Wallets" artboard="Wallet" /> },
      { path: '/wallets/:address', element: <Placeholder title="Wallet" artboard="Wallet" /> },
      { path: '/wallets/:address/positions', element: <Placeholder title="Stake positions" artboard="Positions" /> },
      { path: '/claimable', element: <Placeholder title="Claimable" artboard="Claimable" /> },
      { path: '/activity', element: <Placeholder title="Activity" artboard="Activity" /> },
      { path: '/alerts', element: <Placeholder title="Alerts" artboard="Alerts" /> },
      { path: '/settings', element: <Placeholder title="Settings" artboard="Settings" /> },
      { path: '/help', element: <Placeholder title="Help" artboard="Help" /> },
    ],
  },
]);
