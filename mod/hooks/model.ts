import type { Agent, Check, Group, Note, Pr, Run, Snapshot, Step, Tone } from '../types'

type Raw = Record<string, any>

export const EMPTY: Snapshot = {
  at: 0, red: 0, awaitingReview: 0, prLoading: true, prs: [], agentCount: 0, working: 0, needsYou: 0,
  groups: [], earlier: [], runs: [], runsOn: false, events: [],
}

const SHOWN_CHECKS = 6
const SHOWN_EVENTS = 8

// A PR with nothing to do takes one line, as on the page: a quiet draft, or green with no review needed.
function isQuiet(pr: Raw): boolean {
  const c = pr.counts ?? {}
  if (pr.red || pr.fix || pr.copilotOpen > 0 || (c.running ?? 0) + (c.queued ?? 0) > 0 || pr.merge === 'conflicting') return false
  return !!pr.draft || pr.waiting === 'green'
}

// The colour of a PR's card and the one line saying what it needs, if anything.
function need(pr: Raw): { tone: Tone; why?: string } {
  const c = pr.counts ?? {}
  if (pr.red) return { tone: 'red', why: pr.fix?.why ?? 'checks failing' }
  if (pr.fix && !(pr.fix.onlyBehind && pr.autofix)) return { tone: 'amber', why: pr.fix.why }
  if (pr.merge === 'conflicting') return { tone: 'amber', why: 'merge conflict' }
  if (pr.copilotOpen > 0) return { tone: 'amber', why: `${pr.copilotOpen} Copilot comment${pr.copilotOpen === 1 ? '' : 's'} open` }
  if (pr.waiting === 'reviewer') return { tone: 'green', why: 'green · awaiting review' }
  if (pr.waiting === 'merge') return { tone: 'green', why: 'approved · ready to merge' }
  if ((c.running ?? 0) + (c.queued ?? 0) > 0) return { tone: 'blue' }
  return { tone: 'dim', why: pr.blocker && !pr.draft ? `not merging yet: ${pr.blocker}` : undefined }
}

const CHECK_ORDER: Record<string, number> = { failed: 0, running: 1, queued: 2 }

function pr(p: Raw): Pr {
  const checks: Check[] = (p.checks ?? [])
    .filter((c: Raw) => c.state in CHECK_ORDER)
    .sort((a: Raw, b: Raw) => CHECK_ORDER[a.state]! - CHECK_ORDER[b.state]!)
    .slice(0, SHOWN_CHECKS)
    .map((c: Raw) => ({ label: c.label, state: c.state, startedAt: c.startedAt ?? undefined, usual: c.usual ?? undefined, url: c.url ?? undefined }))
  const repo = String(p.repo ?? '').split('/').pop()
  return {
    url: p.url,
    name: `${repo}#${p.number}`,
    title: p.title ?? '',
    draft: !!p.draft,
    managed: !!p.managed,
    followed: !!p.pasted,
    tag: p.tag ? `${p.tag.story} · ${p.tag.task}` : undefined,
    labels: (p.labels ?? []).slice(0, 4),
    ...need(p),
    quiet: isQuiet(p),
    rail: (p.rail ?? []).map((s: Raw): Step => ({ state: s.state, text: s.text })),
    leftSeconds: p.leftSeconds ?? undefined,
    checks,
    failed: p.counts?.failed ?? 0,
    autofix: !!p.autofix,
    autoreview: !!p.autoreview,
    automerge: p.automerge?.available ? !!p.automerge.on : undefined,
    fixLink: p.fix && !p.fix.onlyBehind ? p.fix.link : undefined,
    agentLink: p.agent?.link ?? undefined,
    updatedAt: p.updatedAt ?? undefined,
  }
}

function note(n: Raw | undefined): Note | undefined {
  if (!n) return undefined
  return { task: n.task ?? '', progress: n.progress ?? '', waiting: n.waiting ?? '', next: n.next ?? '', needsYou: !!n.needsYou, updatedAt: n.updatedAt ?? undefined }
}

// The page's state cut down to what the pane draws.
export function summarize(state: Raw, at: number): Snapshot {
  const a = state.agents ?? {}
  const notes: Raw[] = a.notes ?? []
  const groups: Group[] = (a.groups ?? []).map((g: Raw) => ({
    label: g.label,
    story: !!g.story,
    agents: (g.entries ?? []).map((e: Raw): Agent => ({
      key: e.pane,
      label: e.label,
      source: e.source ?? '',
      state: e.state,
      status: e.status,
      depth: e.depth ?? 0,
      link: e.deepLink ?? undefined,
      note: e.note === null || e.note === undefined ? undefined : note(notes[e.note]),
      subagents: (e.subagents ?? []).map((s: Raw) => ({ id: s.id, kind: s.kind, description: s.description, running: !!s.running, updatedAt: s.updatedAt ?? undefined })),
    })),
  }))
  // Desktop sessions first, as on the page.
  const desktopFirst = (g: Group) => (g.agents.some(x => x.source === 'desktop') ? 0 : 1)
  groups.sort((x, y) => desktopFirst(x) - desktopFirst(y))
  const runs: Run[] = (state.runs?.items ?? []).map((r: Raw) => ({
    key: `${r.repo}/${r.id}`,
    id: Number(r.id ?? 0),
    fullRepo: String(r.repo ?? ''),
    repo: String(r.repo ?? '').split('/').pop() ?? '',
    name: r.name ?? '',
    title: r.title ?? '',
    state: r.state,
    url: r.url,
    watched: !!r.watched,
    startedAt: r.startedAt ?? r.createdAt ?? undefined,
    updatedAt: r.updatedAt ?? undefined,
    jobs: (r.jobs ?? []).map((j: Raw) => ({ name: j.name, state: j.state })),
  }))
  return {
    at,
    red: state.red ?? 0,
    awaitingReview: state.awaitingReview ?? 0,
    prLoading: !!state.loading,
    prUpdated: state.prUpdated ?? undefined,
    prError: state.prError ?? undefined,
    prs: (state.prs ?? []).map(pr),
    agentCount: a.count ?? 0,
    working: a.working ?? 0,
    needsYou: a.needsYou ?? 0,
    groups,
    earlier: (a.earlier ?? []).map((i: number) => note(notes[i])).filter((n: Note | undefined): n is Note => !!n),
    runs,
    runsOn: !!state.runs?.on,
    events: (state.events ?? []).slice(0, SHOWN_EVENTS).map((e: Raw) => ({ at: e.at, text: e.text })),
    fiveHour: typeof state.usage?.fiveHour?.pct === 'number' ? { pct: state.usage.fiveHour.pct, resetsAt: state.usage.fiveHour.resetsAt } : undefined,
    sevenDay: typeof state.usage?.sevenDay?.pct === 'number' ? { pct: state.usage.sevenDay.pct, resetsAt: state.usage.sevenDay.resetsAt } : undefined,
  }
}

/** This session's own plan usage, live, over capcom web's copy, which is only as fresh as the last CLI status line. */
export function withLive(s: Snapshot, limits: { kind: string; percentUsed: number; resetsAt?: string }[]): Snapshot {
  const of = (kind: string) => {
    const l = limits.find(x => x.kind === kind)
    return l ? { pct: l.percentUsed, resetsAt: l.resetsAt } : undefined
  }
  return { ...s, fiveHour: of('five_hour') ?? s.fiveHour, sevenDay: of('seven_day') ?? s.sevenDay }
}

/** A segmented gauge, `▮▮▮▯▯`, split so each half takes its own colour. */
export function gauge(pct: number, cells: number): { on: string; off: string } {
  const n = Math.max(0, Math.min(cells, Math.round((pct / 100) * cells)))
  return { on: '▮'.repeat(n), off: '▯'.repeat(cells - n) }
}

/** How far through the working week it is: Monday to Friday, 06:00 to 17:00 local time. */
export function weekPct(now: Date): number {
  const day = now.getDay()
  if (day === 0 || day === 6) return 100
  const hours = Math.min(11, Math.max(0, now.getHours() + now.getMinutes() / 60 - 6))
  return Math.round((((day - 1) * 11 + hours) / 55) * 100)
}

export function ago(iso: string | undefined, now: number): string {
  const at = iso ? Date.parse(iso) : NaN
  if (Number.isNaN(at)) return ''
  const s = Math.max(0, Math.round((now - at) / 1000))
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}m`
  if (s < 86400) return `${Math.floor(s / 3600)}h`
  return `${Math.floor(s / 86400)}d`
}

export function until(iso: string | undefined, now: number): string {
  const at = iso ? Date.parse(iso) : NaN
  if (Number.isNaN(at)) return ''
  const min = Math.max(0, Math.round((at - now) / 60000))
  return min >= 60 ? `${Math.floor(min / 60)}h${String(min % 60).padStart(2, '0')}` : `${min}m`
}

export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000))
  const m = Math.floor(s / 60)
  return m < 60 ? `${m}:${String(s % 60).padStart(2, '0')}` : `${Math.floor(m / 60)}h${String(m % 60).padStart(2, '0')}`
}

export function approx(seconds: number): string {
  return seconds < 90 ? `${Math.max(1, Math.round(seconds))}s` : `${Math.round(seconds / 60)}m`
}

export function shorten(text: string, max: number): string {
  return text.length <= max ? text : `${text.slice(0, Math.max(1, max - 1))}…`
}
