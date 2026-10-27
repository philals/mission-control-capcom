export type Tone = 'red' | 'amber' | 'green' | 'blue' | 'dim'

export type Step = { state: 'done' | 'going' | 'wrong' | 'pending'; text: string }

export type Check = { label: string; state: string; startedAt?: string; usual?: number; url?: string }

export type Pr = {
  url: string
  name: string
  title: string
  draft: boolean
  managed: boolean
  followed: boolean
  tag?: string
  labels: string[]
  tone: Tone
  why?: string
  quiet: boolean
  rail: Step[]
  leftSeconds?: number
  checks: Check[]
  failed: number
  autofix: boolean
  autoreview: boolean
  automerge?: boolean
  fixLink?: string
  agentLink?: string
  updatedAt?: string
}

export type Subagent = { id: string; kind: string; description: string; running: boolean; updatedAt?: string }

export type Note = { task: string; progress: string; waiting: string; next: string; needsYou: boolean; updatedAt?: string }

export type Agent = { key: string; label: string; source: string; state: string; status: string; depth: number; link?: string; note?: Note; subagents: Subagent[] }

export type Group = { label: string; story: boolean; agents: Agent[] }

export type Run = { key: string; id: number; fullRepo: string; repo: string; name: string; title: string; state: string; url: string; watched: boolean; startedAt?: string; updatedAt?: string; jobs: { name: string; state: string }[] }

export type Gauge = { pct: number; resetsAt?: string }

export type Snapshot = {
  /** When capcom web last answered, in ms since 1970; 0 before the first answer. */
  at: number
  /** Why the last ask failed, if it did. */
  error?: string
  red: number
  awaitingReview: number
  prLoading: boolean
  prUpdated?: string
  prError?: string
  prs: Pr[]
  agentCount: number
  working: number
  needsYou: number
  groups: Group[]
  earlier: Note[]
  runs: Run[]
  runsOn: boolean
  events: { at: string; text: string }[]
  fiveHour?: Gauge
  sevenDay?: Gauge
}

/** An action that asks first: what it does, and what to send once confirmed. */
export type Pending = { key: string; question: string; action: string; args: Record<string, string | number | boolean> }

export type View = {
  /** PRs shown the other way from their default: quiet ones start collapsed, the rest open. */
  flipped: string[]
  pending?: Pending
  showEarlier: boolean
  showLog: boolean
  message?: string
}

declare module 'claude-code' {
  interface PluginState {
    capcom: { snapshot: Snapshot; view: View }
  }
}
