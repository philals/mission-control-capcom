import { atom, read, update } from 'claude-code'
import type { Register } from 'claude-code'

import type { AgentLine, Gauge, PrLine, Snapshot, Tone } from '../types'

const BASE = 'http://127.0.0.1:1969'
const PANE = 'capcom'
const EVERY_MS = 5000
const EMPTY: Snapshot = { at: 0, red: 0, awaitingReview: 0, prCount: 0, prs: [], agentsWorking: 0, agents: [] }
const snapshot = atom({ plugin: 'capcom', key: 'snapshot' } as const, EMPTY)

const COLOR: Record<Tone, string> = { red: 'error', orange: 'warning', green: 'success', cyan: 'suggestion', dim: 'inactive' }

type Raw = Record<string, any>

// What a PR needs, if anything: the PRs with nothing to do are left out.
function prNeed(pr: Raw): { why: string; tone: Tone } | undefined {
  if (pr.red) return { why: pr.fix?.why ?? 'checks failing', tone: 'red' }
  if (pr.fix && !(pr.fix.onlyBehind && pr.autofix)) return { why: pr.fix.why, tone: 'orange' }
  if (pr.merge === 'conflicting') return { why: 'merge conflict', tone: 'orange' }
  if (pr.copilotOpen > 0) return { why: `${pr.copilotOpen} Copilot comment${pr.copilotOpen === 1 ? '' : 's'}`, tone: 'orange' }
  if (pr.waiting === 'reviewer') return { why: 'awaiting review', tone: 'green' }
  if (pr.waiting === 'merge') return { why: 'approved, ready to merge', tone: 'green' }
  return undefined
}

function gauge(g: Raw | undefined): Gauge | undefined {
  return typeof g?.pct === 'number' ? { pct: g.pct, resetsAt: g.resetsAt } : undefined
}

// The page's state cut down to what the pane draws.
export function summarize(state: Raw, at: number): Snapshot {
  const prs: PrLine[] = []
  for (const pr of state.prs ?? []) {
    const need = prNeed(pr)
    if (!need) continue
    const repo = String(pr.repo ?? '').split('/').pop()
    prs.push({ key: pr.url, name: `${repo}#${pr.number}`, title: pr.title ?? '', url: pr.url, fix: pr.fix?.link, ...need })
  }
  const a = state.agents ?? {}
  const notes: Raw[] = a.notes ?? []
  const agents: AgentLine[] = []
  for (const g of a.groups ?? []) {
    for (const e of g.entries ?? []) {
      if (e.state !== 'needs') continue
      const note = e.note === null || e.note === undefined ? undefined : notes[e.note]
      agents.push({ key: e.pane, label: e.label, ask: note?.next || note?.waiting || note?.progress || 'needs you', link: e.deepLink ?? undefined })
    }
  }
  for (const i of a.earlier ?? []) {
    const note = notes[i]
    if (note?.needsYou) agents.push({ key: `earlier-${note.session}`, label: note.task || 'ended session', ask: note.next || note.waiting || 'needs you' })
  }
  return {
    at,
    red: state.red ?? 0,
    awaitingReview: state.awaitingReview ?? 0,
    prCount: (state.prs ?? []).length,
    prError: state.prError ?? undefined,
    prs,
    agentsWorking: a.working ?? 0,
    agents,
    fiveHour: gauge(state.usage?.fiveHour),
    sevenDay: gauge(state.usage?.sevenDay),
  }
}

function usageTone(pct: number): string {
  return pct >= 85 ? 'error' : pct >= 60 ? 'warning' : 'success'
}

function until(iso: string | undefined, now: number): string {
  const at = iso ? Date.parse(iso) : NaN
  if (Number.isNaN(at)) return ''
  const min = Math.max(0, Math.round((at - now) / 60000))
  return min >= 60 ? ` ${Math.floor(min / 60)}h${String(min % 60).padStart(2, '0')}` : ` ${min}m`
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({ name: 'capcom', description: 'Show the CAPCOM console beside this session' })
    const refresh = async () => {
      const now = await $.clock.now()
      try {
        const res = await $.http.fetch(`${BASE}/api/state`)
        if (!res.ok) throw new Error(`capcom web answered ${res.status}`)
        const value = summarize(JSON.parse(res.text), now)
        await update($, snapshot, () => value)
      } catch (err) {
        const error = err instanceof Error ? err.message : String(err)
        await update($, snapshot, prev => (prev.error === error ? prev : { ...prev, error }))
      }
    }
    void refresh()
    $.clock.every(EVERY_MS, () => void refresh())
    void $.ui.open({ id: PANE, title: 'CAPCOM' })
    return next(e)
  })

  on('command.run', { command: 'capcom' }, async $ => {
    const opened = await $.ui.open({ id: PANE, title: 'CAPCOM', focus: true })
    return { text: opened.isPlaced ? 'CAPCOM is open.' : `CAPCOM could not open: ${opened.reason}` }
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text, Button } = $.ui.resolve(e)
    const s = await read($, snapshot)
    const now = await $.clock.now()
    // capcom web opens links itself (GitHub in your browser, claude:// in the desktop app).
    const open = (url: string) => () => {
      void $.http.fetch(`${BASE}/open`, { method: 'POST', headers: { 'x-capcom': 'open', 'content-type': 'application/json' }, body: JSON.stringify({ url }) })
    }
    const openPage = () => void $.process.run(['xdg-open', BASE])

    if (s.at === 0 || (s.error && now - s.at > 3 * EVERY_MS)) {
      return (
        <Box flexDirection="column" gap={1}>
          <Text color="warning">{s.at === 0 && !s.error ? 'Asking capcom web…' : `capcom web is not answering at ${BASE}`}</Text>
          {s.error && <Text dimColor>{s.error}</Text>}
          {s.error && <Text dimColor>Start it with: systemctl --user start capcom-web</Text>}
        </Box>
      )
    }

    const go = s.red === 0
    return (
      <Box flexDirection="column" gap={1}>
        <Box flexDirection="row" gap={2} flexWrap="wrap">
          <Text bold color={go ? 'success' : 'error'}>{go ? '● GO' : `● NO-GO · ${s.red} red`}</Text>
          {s.fiveHour && <Text color={usageTone(s.fiveHour.pct)}>5H {Math.round(s.fiveHour.pct)}%<Text dimColor>{until(s.fiveHour.resetsAt, now)}</Text></Text>}
          {s.sevenDay && <Text color={usageTone(s.sevenDay.pct)}>7D {Math.round(s.sevenDay.pct)}%<Text dimColor>{until(s.sevenDay.resetsAt, now)}</Text></Text>}
        </Box>

        <Box flexDirection="column">
          <Text bold>PULL REQUESTS <Text dimColor>· {s.prCount} open{s.awaitingReview ? ` · ${s.awaitingReview} awaiting review` : ''}</Text></Text>
          {s.prError && <Text color="error">{s.prError}</Text>}
          {s.prs.length === 0 && <Text dimColor>Nothing needs you.</Text>}
          {s.prs.map(pr => (
            <Box key={pr.key} flexDirection="row" gap={1}>
              <Button key={pr.key} plain onPress={open(pr.url)}>
                <Text color={COLOR[pr.tone]}>●</Text> <Text bold>{pr.name}</Text> <Text color={COLOR[pr.tone]}>{pr.why}</Text> <Text dimColor>{pr.title}</Text>
              </Button>
              {pr.fix && pr.tone !== 'green' && <Button key={`fix-${pr.key}`} label="fix ↗" onPress={open(pr.fix)} />}
            </Box>
          ))}
        </Box>

        <Box flexDirection="column">
          <Text bold>AGENTS <Text dimColor>· {s.agentsWorking} working</Text></Text>
          {s.agents.length === 0 && <Text dimColor>Nobody needs you.</Text>}
          {s.agents.map(a =>
            a.link ? (
              <Button key={a.key} plain onPress={open(a.link)}>
                <Text color="warning">!</Text> <Text bold>{a.label}</Text> <Text color="warning">{a.ask}</Text>
              </Button>
            ) : (
              <Text><Text color="warning">!</Text> <Text bold>{a.label}</Text> <Text color="warning">{a.ask}</Text></Text>
            ),
          )}
        </Box>

        <Button key="page" label="Open CAPCOM ↗" onPress={openPage} />
      </Box>
    )
  })
}
