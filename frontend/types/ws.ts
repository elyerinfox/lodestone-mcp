// TypeScript mirror of the Rust `src/ws.rs` envelope. Kept in lockstep
// by hand — there are only three structs and they change rarely. If a
// field is added on the Rust side, add it here too.

export interface ProviderEntry {
  kind: string
  id: string
}

export interface ServerStatus {
  name: string
  version: string
  uptime_secs: number
  tools_active: number
  tools_disabled: number
  tools_active_names: string[]
  tools_disabled_names: string[]
  tools_runtime_disabled_names: string[]
  providers: ProviderEntry[]
  bind: string
  constellation_bind: string
  secrets: SecretPresence
  log_level: string
}

export interface SecretPresence {
  auth_token: boolean
  network_token: boolean
  github_token: boolean
  nasa_key: boolean
  eia_key: boolean
}

export interface MemoryStats {
  enabled: boolean
  memos: number
  solutions: number
  solution_revisions: number
  solution_tags: number
  solution_links: number
  solution_phrasings: number
  conversations: number
  conversation_turns: number
  synonyms: number
  db_path: string
  embedding_model: string
  auto_recall: boolean
  record_conversations: boolean
}

export interface Capabilities {
  query: boolean
  retrieval: boolean
  blob: boolean
  browser: boolean
}

export interface PeerEntry {
  url: string
  node_id: string | null
  reputation: number
  reachable: boolean
  delegation_enabled: boolean
  known_peers: string[]
  capabilities?: Capabilities
}

export interface ConstellationState {
  enabled: boolean
  node_id: string
  constellation_id: string
  peer_count: number
  peers: PeerEntry[]
  delegation_enabled: boolean
  delegation_max_jobs_per_peer_per_hour: number
  delegation_max_bytes_per_job: number
  delegation_total_bytes_per_hour: number
  total_served_bytes: number
  total_fetched_bytes: number
  local_urls: string[]
  // Runtime-tunable values currently in effect (mirror /api/settings).
  max_peers: number
  min_agreement: number
  // Read-only config-file values for knobs that require a restart
  // to change. The settings drawer surfaces them with a badge.
  mdns_configured: boolean
  sync_secs_configured: number
  request_timeout_ms_configured: number
  local_capabilities: Capabilities
}

export interface Snapshot {
  server: ServerStatus
  memory: MemoryStats
  constellation: ConstellationState
  browser: BrowserState
}

export interface BrowserSession {
  session_id: string
  created_secs_ago: number
  idle_secs: number
  url?: string
  title?: string
}

export type PersonaState = 'healthy' | 'suspect' | 'blocked'

export interface BrowserPersona {
  name: string
  state: PersonaState
  session_id: string | null
  url?: string | null
  last_warning?: string | null
  age_secs: number
}

export interface GuestSession {
  peer_id: string
  persona_name: string
  state: PersonaState
  session_id: string | null
  url?: string | null
  last_warning?: string | null
  age_secs: number
}

export interface BrowserState {
  sessions: BrowserSession[]
  personas: BrowserPersona[]
  guest_sessions: GuestSession[]
  idle_timeout_secs: number
  max_concurrent: number
}

// Tagged envelope: matches `#[serde(tag = "type", content = "data")]`.
// One variant for now (snapshot), but the dashboard pattern-matches on
// `type` so additional variants (e.g. "memo_added") slot in without
// breaking older clients.
export type WsMessage = { type: 'snapshot'; data: Snapshot }

export type ConnectionStatus = 'connecting' | 'open' | 'closed' | 'reconnecting'

// /api/memory/graph payload — mirrors src/skills/memory.rs::GraphSnapshot.
export interface GraphNode {
  id: string
  problem: string
  summary: string
  tags: string[]
  revision_count: number
  updated_at: number
  superseded_by_head: string | null
}

export interface GraphEdge {
  from: string
  to: string
  kind: string
}

export interface MemoryGraph {
  nodes: GraphNode[]
  edges: GraphEdge[]
}

export type GraphMode = 'all' | 'filter' | 'focus'

export interface FilterParams {
  tag?: string
  query?: string
  hide_superseded?: boolean
}

export interface FocusParams {
  id: string
  depth: number
}
