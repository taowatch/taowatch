import { useQuery } from '@tanstack/react-query';
import { Navigate, useLocation } from 'react-router';
import type { ReactNode } from 'react';
import { ApiError, getJson, type Me } from '../lib/api';

export function useMe() {
  return useQuery({
    queryKey: ['me'],
    queryFn: () => getJson<Me>('/api/me'),
    retry: (n, e) => !(e instanceof ApiError && e.status === 401) && n < 2,
    staleTime: 60_000,
  });
}

/** Renders children for signed-in users, otherwise redirects to /login. */
export function RequireAuth({ children }: { children: ReactNode }) {
  const me = useMe();
  const location = useLocation();
  if (me.isPending) {
    return (
      <p role="status" className="page dim">
        Loading…
      </p>
    );
  }
  if (me.isError) {
    if (me.error instanceof ApiError && me.error.status === 401) {
      return <Navigate to="/login" replace state={{ from: location.pathname }} />;
    }
    return (
      <p role="alert" className="page down">
        Could not reach the API: {String(me.error.message)}
      </p>
    );
  }
  return <>{children}</>;
}
