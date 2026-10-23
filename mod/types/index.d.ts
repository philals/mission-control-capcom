export type Tone = 'red' | 'orange' | 'green' | 'cyan' | 'dim'

export type PrLine = { key: string; name: string; title: string; why: string; tone: Tone; url: string; fix?: string }

export type AgentLine = { key: string; label: string; ask: string; link?: string }

export type Gauge = { pct: number; resetsAt?: string }

export type Snapshot = {
  /** When capcom web last answered, in ms since 1970; 0 before the first answer. */
  at: number
  /** Why the last ask failed, if it did. */
  error?: string
  red: number
  awaitingReview: number
  prCount: number
  prError?: string
  prs: PrLine[]
  agentsWorking: number
  agents: AgentLine[]
  fiveHour?: Gauge
  sevenDay?: Gauge
}

declare module 'claude-code' {
  interface PluginState {
    capcom: { snapshot: Snapshot }
  }
}
