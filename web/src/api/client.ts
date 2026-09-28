export type ApiErrorBody = { error?: { code?: string; message?: string; request_id?: string } };

export class ApiError extends Error {
  readonly status: number;
  readonly code?: string;
  readonly requestId?: string;

  constructor(status: number, body: ApiErrorBody) {
    const message = body.error?.message ?? `Request failed (${status})`;
    super(body.error?.request_id ? `${message} · ref ${body.error.request_id.slice(0, 8)}` : message);
    this.name = "ApiError";
    this.status = status;
    this.code = body.error?.code;
    this.requestId = body.error?.request_id;
  }
}

export async function apiRequest<T>(path: string, init: RequestInit = {}, options: { redirectOnUnauthorized?: boolean } = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.body && !headers.has("Content-Type")) headers.set("Content-Type", "application/json");
  if (["POST", "PATCH", "PUT", "DELETE"].includes(init.method?.toUpperCase() ?? "") && !headers.has("Content-Type")) headers.set("X-Sprinter", "1");
  const response = await fetch(path, { ...init, headers, credentials: "same-origin" });
  if (response.status === 401 && options.redirectOnUnauthorized !== false && !path.startsWith("/api/auth/login")) {
    const returnTo = `${window.location.pathname}${window.location.search}${window.location.hash}`;
    window.location.assign(`/login?redirect=${encodeURIComponent(returnTo)}`);
  }
  if (!response.ok) {
    const body = (await response.json().catch(() => ({}))) as ApiErrorBody;
    if (response.status >= 500 && path !== "/api/client-log") {
      void import("../clientErrors").then(({ reportClientError }) => reportClientError({
        level: "error",
        message: `API request failed with status ${response.status}${body.error?.request_id ? ` · ref ${body.error.request_id}` : ""}`,
        route: path.split("?")[0],
        stack: new Error().stack,
      })).catch(() => undefined);
    }
    throw new ApiError(response.status, body);
  }
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export function jsonBody(value: unknown): string {
  return JSON.stringify(value);
}
