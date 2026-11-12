import {SessionProvider} from '../features/terminal/session-provider';
import Workbench from '../features/workbench/workbench';
import {TransferProvider} from '../features/files/transfer-provider';
import {ExecutionProvider} from '../features/ai/execution-feed';
import {QuickCommandDraftProvider} from '../features/quick-commands/command-draft';

export function RootLayout() {
  return <SessionProvider><TransferProvider><ExecutionProvider><QuickCommandDraftProvider><Workbench /></QuickCommandDraftProvider></ExecutionProvider></TransferProvider></SessionProvider>;
}
