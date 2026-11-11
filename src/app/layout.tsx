import {SessionProvider} from '../features/terminal/session-provider';
import Workbench from '../features/workbench/workbench';
import {TransferProvider} from '../features/files/transfer-provider';

export function RootLayout() {
  return <SessionProvider><TransferProvider><Workbench /></TransferProvider></SessionProvider>;
}
