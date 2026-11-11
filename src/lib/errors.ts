export type ErrorKind = 'http' | 'network' | 'timeout' | 'invalid-response' | 'storage';

export class AppError extends Error {
  constructor(
    readonly kind: ErrorKind,
    message: string,
    readonly status?: number,
  ) {
    super(message);
    this.name = 'AppError';
  }
}

export function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === 'AbortError';
}

export function errorMessage(error: unknown): string {
  return error instanceof AppError ? error.message : '暂时无法完成操作，请重试。';
}

export function shouldRetry(failureCount: number, error: unknown): boolean {
  if (failureCount >= 2 || !(error instanceof AppError)) return false;
  return error.kind === 'network' || error.kind === 'timeout' ||
    (error.kind === 'http' && (error.status ?? 0) >= 500);
}
