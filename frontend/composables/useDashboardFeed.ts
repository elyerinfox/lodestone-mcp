// Single source of truth for the `/ws/status` feed. Returns reactive
// refs the dashboard pages bind directly. Auto-reconnects on close so a
// rolling backend restart resumes the live view without manual refresh.

import { ref } from 'vue'
import { useWebSocket } from '@vueuse/core'
import type { ConnectionStatus, Snapshot, WsMessage } from '~/types/ws'

export function useDashboardFeed() {
  const config = useRuntimeConfig()
  const snapshot = ref<Snapshot | null>(null)
  const status = ref<ConnectionStatus>('connecting')
  const lastError = ref<string | null>(null)
  const lastUpdatedAt = ref<Date | null>(null)

  // Resolve the WebSocket URL. Explicit `NUXT_PUBLIC_WS_URL` wins; else
  // we derive `ws(s)://<host>/ws/status` from the current origin so the
  // dashboard "just works" when same-served.
  const buildUrl = () => {
    const override = (config.public.wsUrl as string).trim()
    const token = (config.public.wsToken as string).trim()
    const base = override.length > 0
      ? override
      : (() => {
          const proto = window.location.protocol === 'https:' ? 'wss' : 'ws'
          return `${proto}://${window.location.host}/ws/status`
        })()
    return token.length > 0
      ? `${base}${base.includes('?') ? '&' : '?'}token=${encodeURIComponent(token)}`
      : base
  }

  // VueUse drives the lifecycle; we just translate inbound payloads
  // into typed snapshots and surface connection status to the UI.
  const { open, close } = useWebSocket(
    () => (typeof window === 'undefined' ? '' : buildUrl()),
    {
      autoReconnect: {
        retries: -1,        // forever
        delay: 1500,
        onFailed: () => {
          status.value = 'closed'
          lastError.value = 'reconnect retries exhausted'
        },
      },
      onConnected: () => {
        status.value = 'open'
        lastError.value = null
      },
      onDisconnected: () => {
        status.value = 'reconnecting'
      },
      onError: (_ws, event) => {
        lastError.value = 'WebSocket error (see browser console)'
        // event is a generic Event in browsers; nothing useful to surface.
        console.warn('[ws] error event:', event)
      },
      onMessage: (_ws, event) => {
        try {
          const parsed: WsMessage = JSON.parse(event.data)
          if (parsed.type === 'snapshot') {
            snapshot.value = parsed.data
            lastUpdatedAt.value = new Date()
          }
        } catch (e) {
          lastError.value = `parse failed: ${e instanceof Error ? e.message : 'unknown'}`
        }
      },
    },
  )

  return {
    snapshot,
    status,
    lastError,
    lastUpdatedAt,
    open,
    close,
  }
}
