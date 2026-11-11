import {AppError} from './errors';

type JsonOptions<T> = Omit<RequestInit, 'signal'> & {
  signal?: AbortSignal | undefined;
  timeoutMs?: number;
  parse: (value: unknown) => T;
};

function httpMessage(status: number): string {
  if (status === 401) return '服务器要求身份验证，请联系管理员确认接入方式。';
  if (status === 403) return '当前请求没有访问权限。';
  if (status === 404) return '没有找到请求的内容。';
  if (status === 409) return '内容已发生变化，请刷新后重试。';
  if (status === 429) return '请求过于频繁，请稍后重试。';
  if (status >= 500) return '服务暂时不可用，请稍后重试。';
  return '服务器未接受这次请求，请检查填写内容。';
}

/** One transport boundary: cancellation, timeout, HTTP status, JSON and schema validation. */
export async function requestJson<T>(url: string, options: JsonOptions<T>): Promise<T> {
  const {parse, timeoutMs = 15_000, signal, ...init} = options;
  const controller = new AbortController();
  let timedOut = false;
  const cancel = () => controller.abort(signal?.reason);
  if (signal?.aborted) cancel();
  else signal?.addEventListener('abort', cancel, {once: true});
  const timer = setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, timeoutMs);
  const headers = new Headers(init.headers);
  headers.set('Accept', 'application/json');

  try {
    const response = await fetch(url, {...init, headers, signal: controller.signal});
    if (!response.ok) throw new AppError('http', httpMessage(response.status), response.status);
    let value: unknown;
    try {
      value = JSON.parse(await response.text()) as unknown;
    } catch (error) {
      if (controller.signal.aborted) throw error;
      throw new AppError('invalid-response', '服务器返回了无法识别的数据。');
    }
    try {
      return parse(value);
    } catch (error) {
      if (error instanceof AppError) throw error;
      throw new AppError('invalid-response', '返回的数据格式与应用要求不一致。');
    }
  } catch (error) {
    if (signal?.aborted) throw new DOMException('Request cancelled', 'AbortError');
    if (timedOut) throw new AppError('timeout', '请求超时，请检查连接后重试。');
    if (error instanceof AppError) throw error;
    throw new AppError('network', '无法连接服务，请检查网络和服务地址。');
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener('abort', cancel);
  }
}
