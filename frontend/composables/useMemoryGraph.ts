// Tiny fetch wrapper for /api/memory/graph. Sends the bearer token when
// one is configured (same convention the WS feed and other settings
// endpoints use). Returns the typed payload; throws on non-2xx.
//
// The API endpoint lives on the MCP server. When the dashboard is
// embedded inside the MCP binary it's same-origin. When the dashboard
// runs as its own container, the MCP server is at a DIFFERENT origin
// (different port, possibly different host). We derive the API origin
// from the WebSocket URL baked in at build time, since both feeds live
// on the same MCP listener.

import { useRuntimeConfig } from '#app'
import type {
  FilterParams,
  FocusParams,
  GraphMode,
  MemoryGraph,
} from '~/types/ws'

/// Convert ws:// or wss:// to http:// or https://, stripping the path
/// so we keep just the origin. Falls back to same-origin when the
/// config is empty (the embedded-into-the-binary case).
function deriveApiOrigin(wsUrl: string): string {
  if (!wsUrl) return window.location.origin
  try {
    const u = new URL(wsUrl)
    const httpProto = u.protocol === 'wss:' ? 'https:' : 'http:'
    return `${httpProto}//${u.host}`
  } catch {
    return window.location.origin
  }
}

export function useMemoryGraph() {
  const cfg = useRuntimeConfig()
  const token = (cfg.public.wsToken as string | undefined) ?? ''
  const wsUrl = (cfg.public.wsUrl as string | undefined) ?? ''

  async function fetchGraph(
    mode: GraphMode,
    params: FilterParams | FocusParams | undefined = undefined,
  ): Promise<MemoryGraph> {
    const origin = deriveApiOrigin(wsUrl)
    const url = new URL('/api/memory/graph', origin)
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
