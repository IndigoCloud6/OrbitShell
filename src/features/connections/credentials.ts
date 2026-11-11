import {useQuery} from '@tanstack/react-query';
import {desktopCommand, isDesktop} from '../../shared/desktop';

export interface CredentialStatus {available: boolean; saved: boolean}
export const credentialStatusKey = (id: string | null) => ['connection-credential', id] as const;

export function useCredentialStatus(profileId: string | null, enabled = true) {
  return useQuery({
    queryKey: credentialStatusKey(profileId),
    queryFn: () => desktopCommand<CredentialStatus>('credential_status', {profileId}),
    enabled: isDesktop && enabled,
    staleTime: 0, gcTime: 0, retry: false,
  });
}

// Only status enters Query's cache. Passwords are sent directly to the native vault.
export function saveCredential(profileId: string, credential: string) {
  return desktopCommand<void>('save_connection_credential', {profileId, credential});
}
export function forgetCredential(profileId: string) {
  return desktopCommand<void>('forget_connection_credential', {profileId});
}
