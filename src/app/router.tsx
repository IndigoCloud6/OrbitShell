import {createHashRouter} from 'react-router';
import {RouteError} from './route-error';

export const router = createHashRouter([
  ...(import.meta.env.DEV ? [{path: '/design-review', lazy: async () => ({Component: (await import('../features/workbench/design-review')).default})}] : []),
  {
  path: '/',
  lazy: async () => ({Component: (await import('./layout')).RootLayout}),
  errorElement: <RouteError />,
  children: [
    {index: true, element: null},
    {path: 'connections', lazy: async () => ({Component: (await import('../features/connections/dialogs')).ConnectionManagerRoute})},
    {path: 'connections/new', lazy: async () => ({Component: (await import('../features/connections/dialogs')).ConnectionEditorRoute})},
    {path: 'connections/:connectionId/edit', lazy: async () => ({Component: (await import('../features/connections/dialogs')).ConnectionEditorRoute})},
    {path: 'connections/:connectionId/connect', lazy: async () => ({Component: (await import('../features/connections/dialogs')).CredentialsRoute})},
    {path: 'settings', lazy: async () => ({Component: (await import('../pages/settings-dialog')).default})},
    {path: 'commands', lazy: async () => ({Component: (await import('../features/quick-commands/quick-commands')).QuickCommandManagerRoute})},
    {path: 'commands/new', lazy: async () => ({Component: (await import('../features/quick-commands/quick-commands')).QuickCommandEditorRoute})},
    {path: 'commands/:commandId/edit', lazy: async () => ({Component: (await import('../features/quick-commands/quick-commands')).QuickCommandEditorRoute})},
    {path: '*', lazy: async () => ({Component: (await import('../pages/not-found-dialog')).default})},
  ],
}]);
