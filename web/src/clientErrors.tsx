import { Component, type ErrorInfo, type ReactNode } from "react";

type ClientError = {
  level: "error" | "warn" | "info";
  message: string;
  stack?: string;
  route?: string;
};

const APP_VERSION = import.meta.env.VITE_APP_VERSION ?? "0.1.0";
let handlersInstalled = false;

function redact(value: string) {
  return value
    .replace(/sk-or-[A-Za-z0-9_-]+/g, "[REDACTED]")
    .replace(/(authorization\s*[:=]\s*bearer\s+)\S+/gi, "$1[REDACTED]")
    .replace(/((?:password|api[_ -]?key|token)\s*[:=]\s*)\S+/gi, "$1[REDACTED]")
    .slice(0, 500);
}

export function reportClientError(error: ClientError) {
  if (typeof window === "undefined" || !navigator.onLine) return;
  const message = redact(error.message || "Unknown client error");
  const stack = error.stack ? redact(error.stack) : undefined;
  const body = JSON.stringify({
    level: error.level,
    message,
    ...(stack ? { stack } : {}),
    route: (error.route ?? window.location.pathname).slice(0, 512),
    app_version: APP_VERSION,
  });
  if (new Blob([body]).size > 8 * 1024) return;
  void fetch("/api/client-log", {
    method: "POST",
    credentials: "same-origin",
    headers: { "Content-Type": "application/json" },
    body,
    keepalive: true,
  }).catch(() => undefined);
}

export function installGlobalErrorHandlers() {
  if (handlersInstalled || typeof window === "undefined") return;
  handlersInstalled = true;
  window.addEventListener("error", (event) => {
    const error = event.error instanceof Error ? event.error : undefined;
    reportClientError({
      level: "error",
      message: error?.message ?? event.message ?? "Uncaught browser error",
      stack: error?.stack,
    });
  });
  window.addEventListener("unhandledrejection", (event) => {
    const reason = event.reason;
    reportClientError({
      level: "error",
      message: reason instanceof Error ? reason.message : String(reason ?? "Unhandled promise rejection"),
      stack: reason instanceof Error ? reason.stack : undefined,
    });
  });
}

type BoundaryProps = { children: ReactNode };
type BoundaryState = { failed: boolean };

export class ClientErrorBoundary extends Component<BoundaryProps, BoundaryState> {
  state: BoundaryState = { failed: false };

  static getDerivedStateFromError(): BoundaryState {
    return { failed: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    reportClientError({
      level: "error",
      message: error.message || "React render failed",
      stack: `${error.stack ?? ""}\n${info.componentStack ?? ""}`,
    });
  }

  render() {
    if (this.state.failed) {
      return <main className="route-loading" role="alert"><div><h1>Something went wrong</h1><p>Reload Sprinter to continue.</p><button type="button" onClick={() => window.location.reload()}>Reload</button></div></main>;
    }
    return this.props.children;
  }
}
