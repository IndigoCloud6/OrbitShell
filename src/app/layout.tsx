import {SessionProvider} from '../features/terminal/session-provider';
import Workbench from '../features/workbench/workbench';
import {TransferProvider} from '../features/files/transfer-provider';
import {AiWorkspaceProvider} from '../features/ai/ai-workspace';
import {QuickCommandDraftProvider} from '../features/quick-commands/command-draft';

export function RootLayout() {
  return <SessionProvider><TransferProvider><AiWorkspaceProvider><QuickCommandDraftProvider><Workbench /></QuickCommandDraftProvider></AiWorkspaceProvider></TransferProvider></SessionProvider>;
}
