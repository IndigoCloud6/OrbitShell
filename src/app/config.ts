const dataMode = import.meta.env.VITE_DATA_MODE || 'demo';
const routerMode = import.meta.env.VITE_ROUTER_MODE || 'history';

if (dataMode !== 'demo' && dataMode !== 'api') {
  throw new Error('VITE_DATA_MODE must be demo or api.');
}
if (routerMode !== 'history' && routerMode !== 'hash') {
  throw new Error('VITE_ROUTER_MODE must be history or hash.');
}

export const config = {
  dataMode,
  routerMode,
  apiBase: (import.meta.env.VITE_API_BASE || '/api').replace(/\/$/, ''),
  basePath: import.meta.env.BASE_URL,
} as const;
