// reagent's API from the browser; the session cookie authenticates it.

export class ApiError extends Error {
  constructor(message, status) {
    super(message);
    this.status = status;
  }
}

async function req(method, path, body) {
  const r = await fetch(path, {
    method,
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    credentials: "same-origin",
  });
  const data = await r.json().catch(() => null);
  if (!r.ok) {
    if (r.status === 401) window.dispatchEvent(new Event("reagent-logged-out"));
    throw new ApiError(data?.error || r.statusText, r.status);
  }
  return data;
}

export const get = (p) => req("GET", p);
export const post = (p, b = {}) => req("POST", p, b);
export const put = (p, b) => req("PUT", p, b);
export const del = (p) => req("DELETE", p);

/** Live events (task changes, notifications, agent events); returns a stop function. */
export function subscribe(onEvent) {
  const es = new EventSource("/api/events");
  es.onmessage = (e) => {
    try {
      onEvent(JSON.parse(e.data));
    } catch {
      /* one bad event isn't worth stopping for */
    }
  };
  return () => es.close();
}
