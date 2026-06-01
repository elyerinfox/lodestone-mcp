// Tiny fetch wrapper for /api/memory/graph. Sends the bearer token when
// one is configured (same convention the WS feed and other settings
// endpoints use). Returns the typed payload; throws on non-2xx.

import { useRuntimeConfig } from '#app'
import type {
  FilterParams,
  FocusParams,
  GraphMode,
  MemoryGraph,
} from '~/types/ws'

export function useMemoryGraph() {
  const cfg = useRuntimeConfig()
  const token = (cfg.public.wsToken as string | undefined) ?? ''

  async function fetchGraph(
    mode: GraphMode,
    params: FilterParams | FocusParams | undefined = undefined,
  ): Promise<MemoryGraph> {
    const url = new URL('/api/memory/graph', window.location.origin)
    url.searchParams.set('mode', mode)
    if (params) {
      for (const [k, v] of Object.entries(params)) {
        if (v === undefined || v === null || v === '') continue
        url.searchParams.set(k, String(v))
      }
    }
    const headers: Record<string, string> = {}
    if (token) headers.Authorization = `Bearer ${token}`
    const res = await fetch(url.toString(), { headers })
    if (!res.ok) {
      const detail = await res.text().catch(() => '')
      throw new Error(`${res.status} ${res.statusText}: ${detail}`.trim())
    }
    return (await res.json()) as MemoryGraph
  }

  return { fetchGraph }
}
