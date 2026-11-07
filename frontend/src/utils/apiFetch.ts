import { API_BASE } from './api';

/**
 * The single place the browser talks to the backend.
 *
 * Every `/api` request now requires a session cookie. When one is missing or
 * expired the backend answers `401` — and a `401` in a browser is not something a
 * component can render its way out of, so this wrapper sends the user to the
 * identity provider instead. Centralising it is what keeps that behaviour uniform:
 * a component that reached `fetch` directly would silently render an empty panel
 * where a sign-in prompt belongs.
 *
 * JavaScript never reads, stores, or sends a token. The session cookie is
 * `HttpOnly`, so the browser attaches it and nothing in this bundle can exfiltrate
 * it — an invariant, not a preference.
 *
 * `API_BASE` comes from `utils/api.ts` rather than being retyped here: the Vite dev
 * proxy and the production nginx location both key off that exact string.
 */
export async function apiFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const response = await fetch(input, init);
  if (response.status === 401) {
    // Return the response as well: callers that check `res.ok` still see a
    // consistent failure, and the redirect is already in flight.
    window.location.assign('/auth/login');
  }
  return response;
}

export { API_BASE };
