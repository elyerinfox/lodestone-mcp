// Tiny wrapper around fetch() for the /api/settings/* endpoints. Pulls
// the same token the WS feed uses (from runtime config), so the
// dashboard only needs to be configured in one place. Returns the
// applied state on success; throws on non-2xx so callers can show an
// error banner.

import { useRuntimeConfig } from '#app'

export function useSettingsApi() {
  const cfg = useRuntimeConfig()
  const token = (cfg.public.wsToken as string | undefined) ?? ''

  async function patch<T = unknown>(subsystem: string, body: Record<string, unknown>): Promise<T> {
    const headers: Record<string, string> = { 'Content-Type': 'application/json' }
    if (token) headers.Authorization = `Bearer ${token}`
    const res = await fetch(`/api/settings/${subsystem}`, {
      method: 'POST',
      headers,
      body: JSON.stringify(body),
    })
    if (!res.ok) {
      const detail = await res.text().catch(() => '')
      throw new Error(`${res.status} ${res.statusText}: ${detail}`.trim())
    }
    return (await res.json()) as T
  }

  return { patch }
}
