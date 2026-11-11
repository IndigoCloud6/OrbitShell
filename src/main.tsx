import {StrictMode} from 'react';
import {createRoot} from 'react-dom/client';
import {RouterProvider} from 'react-router/dom';
import {QueryClient, QueryClientProvider} from '@tanstack/react-query';
import {InternationalizationProvider} from '@astryxdesign/core/i18n';
import zhCN from '@astryxdesign/core/locales/zh-CN.generated.js';
import {LinkProvider} from '@astryxdesign/core/Link';
import {AppTheme} from './app/theme';
import {PreferencesProvider} from './app/preferences';
import {router} from './app/router';
import {RouterLink} from './app/router-link';
import {shouldRetry} from './lib/errors';
import '@astryxdesign/core/reset.css';
import '@astryxdesign/core/astryx.css';
import '@xterm/xterm/css/xterm.css';
import './workbench.css';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {staleTime: 30_000, gcTime: 300_000, retry: shouldRetry, refetchOnWindowFocus: false},
    mutations: {retry: false},
  },
});

const root = document.getElementById('root');
if (!root) throw new Error('The root element is missing.');

createRoot(root).render(
  <StrictMode>
    <InternationalizationProvider locale="zh-CN" messages={{'zh-CN': zhCN}}>
      <AppTheme>
        <LinkProvider component={RouterLink}>
          <QueryClientProvider client={queryClient}>
            <PreferencesProvider><RouterProvider router={router} /></PreferencesProvider>
          </QueryClientProvider>
        </LinkProvider>
      </AppTheme>
    </InternationalizationProvider>
  </StrictMode>,
);
