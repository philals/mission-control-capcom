import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register, RenderChildren, SessionRateLimit } from 'claude-code'

import type { Agent, Check, Pr, Run, Step, Subagent, Tone, View } from '../types'
import { EMPTY, ago, approx, clock, gauge, shorten, summarize, until, weekPct, withLive } from './model'

export { summarize } from './model'

const BASE = 'http://127.0.0.1:1969'
const PANE = 'capcom'
const EVERY_MS = 4000
const snapshot = atom({ plugin: 'capcom', key: 'snapshot' } as const, EMPTY)
const view = atom({ plugin: 'capcom', key: 'view' } as const, { flipped: [], showEarlier: false, showLog: true } as View)

const C = { go: 'success', red: 'error', amber: 'warning', blue: 'suggestion', purple: 'merged', accent: 'claude', dim: 'inactive', faint: 'subtle', text: 'text' }
const TONE: Record<Tone, string> = { red: C.red, amber: C.amber, green: C.go, blue: C.blue, dim: C.faint }
const STEP: Record<Step['state'], string> = { done: C.go, going: C.blue, wrong: C.red, pending: C.dim }
const RUN: Record<string, [string, string]> = {
  running: ['◐', C.blue], queued: ['◌', C.amber], waiting: ['◌', C.purple], success: ['✓', C.go],
  failed: ['✗', C.red], cancelled: ['⊘', C.dim], skipped: ['·', C.dim],
}

let limits: SessionRateLimit[] = []

async function refresh($: EngineInterface) {
  const now = await $.clock.now()
  try {
    const res = await $.http.fetch(`${BASE}/api/state`)
    if (!res.ok) throw new Error(`capcom web answered ${res.status}`)
    const value = withLive(summarize(JSON.parse(res.text), now), limits)
    await update($, snapshot, () => value)
  } catch (err) {
    const error = err instanceof Error ? err.message : String(err)
    await update($, snapshot, prev => (prev.error === error ? prev : { ...prev, error }))
  }
}

// capcom web's own copy of the plan usage, in the status line's shape, so the page has it fresh too (docs/web.md).
async function share($: EngineInterface) {
  if (!limits.length) return
  const windows = Object.fromEntries(limits.map(l => [l.kind, { used_percentage: l.percentUsed, resets_at: l.resetsAt }]))
  try {
    const state = (await $.env.get('XDG_STATE_HOME')) ?? `${await $.env.get('HOME')}/.local/state`
    const file = (await $.env.get('CAPCOM_USAGE_FILE')) ?? `${state}/capcom/usage.json`
    const at = Math.floor((await $.clock.now()) / 1000)
    await $.fs.write(`${file}.tmp`, JSON.stringify({ at, rate_limits: windows }))
    await $.process.run(['mv', `${file}.tmp`, file])
  } catch {}
}

// capcom web opens links itself: GitHub in your browser, claude:// in the desktop app.
async function open($: EngineInterface, url: string) {
  await $.http.fetch(`${BASE}/open`, { method: 'POST', headers: { 'x-capcom': 'open', 'content-type': 'application/json' }, body: JSON.stringify({ url }) })
}

// The same actions the page takes, through the same endpoint and rules.
async function act($: EngineInterface, action: string, args: Record<string, unknown>) {
  let message: string
  try {
    const res = await $.http.fetch(`${BASE}/api/do`, { method: 'POST', headers: { 'x-capcom': 'do', 'content-type': 'application/json' }, body: JSON.stringify({ action, ...args }) })
    message = res.ok ? (JSON.parse(res.text).message ?? 'done') : `refused (${res.status})`
  } catch (err) {
    message = `could not reach capcom web: ${err instanceof Error ? err.message : String(err)}`
  }
  await update($, view, v => ({ ...v, message }))
  await refresh($)
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({ name: 'capcom', description: 'Show the CAPCOM console beside this session' })
    limits = await $.session.usage().then(u => u.rateLimits, () => [])
    void share($)
    void refresh($)
    $.clock.every(EVERY_MS, () => void refresh($))
    void $.ui.open({ id: PANE, title: 'CAPCOM' })
    return next(e)
  })

  on('session.measure', async ($, e, next) => {
    limits = e.rateLimits
    await update($, snapshot, s => withLive(s, limits))
    if (e.changed.includes('rateLimits')) void share($)
    return next(e)
  })

  on('command.run', { command: 'capcom' }, async $ => {
    const opened = await $.ui.open({ id: PANE, title: 'CAPCOM', focus: true })
    return { text: opened.isPlaced ? 'CAPCOM is open.' : `CAPCOM could not open: ${opened.reason}` }
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const els = $.ui.resolve(e)
    const { Box, Text, Button } = els
    const [s, v, now] = await Promise.all([read($, snapshot), read($, view), $.clock.now()])
    const W = Math.max(40, e.props.bodyColumns)
    const inner = W - 4

    const press = (fn: () => Promise<unknown>) => () => void fn()
    const flipped = v.flipped ?? []
    const isOpen = (pr: Pr) => pr.quiet === flipped.includes(pr.url)
    const toggle = (url: string) =>
      press(() => update($, view, x => {
        const f = x.flipped ?? []
        return { ...x, flipped: f.includes(url) ? f.filter(u => u !== url) : [...f, url] }
      }))

    const timer = (key: string, since: string | undefined, color: string, usual?: number) => {
      const at = since ? Date.parse(since) : NaN
      if (Number.isNaN(at)) return null
      return 'Client' in els ? (
        <els.Client key={key} module="./elapsed.tsx" props={usual ? { since: at, now, usual, color } : { since: at, now, color }} />
      ) : (
        <Text color={color}>{clock(now - at) + (usual ? ` of ~${approx(usual)}` : '')}</Text>
      )
    }

    const panel = (color: string, title: string, right: string | null, body: RenderChildren[], dim = false) => (
      <Box flexDirection="column" borderStyle="round" borderColor={color} borderDimColor={dim} paddingX={1} width={W}>
        <Box justifyContent="space-between">
          <Text color={color} bold>{title}</Text>
          {right ? <Text dimColor>{right}</Text> : null}
        </Box>
        {body}
      </Box>
    )

    if (s.at === 0 || (s.error && now - s.at > 3 * EVERY_MS)) {
      return panel(C.amber, 'CAPCOM · NO SIGNAL', null, [
        <Text color={C.amber}>{s.at === 0 && !s.error ? 'Asking capcom web…' : `capcom web is not answering at ${BASE}`}</Text>,
        s.error ? <Text dimColor wrap="truncate">{s.error}</Text> : null,
        s.error ? <Text dimColor>Start it: systemctl --user start capcom-web</Text> : null,
      ])
    }

    const go = s.red === 0
    // ---- mission: the go/no-go poll, plan usage and the working week
    const meter = (label: string, pct: number, reset?: string) => {
      const g = gauge(pct, 5)
      const color = pct >= 85 ? C.red : pct >= 60 ? C.amber : C.blue
      const r = reset ? until(reset, now) : ''
      return (
        <Text>
          <Text dimColor>{`${label} `}</Text>
          <Text color={color}>{g.on}</Text>
          <Text color={C.faint}>{g.off}</Text>
          <Text bold>{` ${Math.round(pct)}%`}</Text>
          {r ? <Text dimColor>{` ↻${r}`}</Text> : null}
        </Text>
      )
    }
    const mission = panel(go ? C.go : C.red, go ? '● GO' : `● NO-GO · ${s.red} red`, s.prLoading ? 'reading PRs…' : s.prUpdated ? `PRs read ${ago(s.prUpdated, now)} ago` : null, [
      <Box columnGap={2} flexWrap="wrap">
        {s.fiveHour ? meter('5h', s.fiveHour.pct, s.fiveHour.resetsAt) : null}
        {s.sevenDay ? meter('7d', s.sevenDay.pct, s.sevenDay.resetsAt) : null}
        {meter('week', weekPct(new Date(now)))}
      </Box>,
      s.prError ? <Text color={C.red} wrap="truncate">{s.prError}</Text> : null,
      v.message ? <Text dimColor wrap="truncate">{`» ${v.message}`}</Text> : null,
      <Box columnGap={1} flexWrap="wrap">
        <Button key="refresh" label="↻ refresh" onPress={press(() => act($, 'refresh', {}))} />
        <Button key="page" label="open CAPCOM ↗" onPress={press(() => $.process.run(['xdg-open', BASE]))} />
      </Box>,
    ])

    // ---- pull requests
    const tick = (key: string, label: string, isOn: boolean, fn: () => void) => (
      <Button key={key} plain dimColor={!isOn} onPress={fn}>
        <Text color={isOn ? C.go : C.dim}>{isOn ? "■" : "□"}</Text> {label}
      </Button>
    )
    const railLine = (pr: Pr) => (
      <Box columnGap={2} flexWrap="wrap">
        {pr.rail.map(st => (
          <Text color={STEP[st.state]}>{`● ${st.text === 'CI running' && pr.leftSeconds ? `CI ${approx(pr.leftSeconds)} left` : st.text}`}</Text>
        ))}
      </Box>
    )
    const checkLine = (pr: Pr, c: Check, i: number) => {
      const [glyph, color] = c.state === 'failed' ? ['✗', C.red] : c.state === 'running' ? ['◐', C.blue] : ['◌', C.amber]
      return (
        <Box justifyContent="space-between">
          <Button key={`check-${pr.url}-${i}`} plain onPress={press(async () => (c.url ? open($, c.url) : undefined))}>
            <Text color={color}>{glyph}</Text> <Text dimColor={c.state !== 'failed'}>{shorten(c.label, Math.max(12, inner - 16))}</Text>
          </Button>
          {c.state === 'running' ? timer(`clock-${pr.url}-${i}`, c.startedAt, C.dim, c.usual) : <Text dimColor>{c.state}</Text>}
        </Box>
      )
    }
    const prStatus = (pr: Pr) =>
      pr.tone === 'blue' ? (pr.leftSeconds ? `CI ${approx(pr.leftSeconds)} left` : 'CI running') : pr.why ?? (pr.draft ? 'draft' : 'green')
    const prCard = (pr: Pr) => {
      const color = TONE[pr.tone]
      if (!isOpen(pr)) {
        const status = prStatus(pr)
        return (
          <Box flexDirection="column" width={W}>
            <Box justifyContent="space-between">
              <Button key={`pr-${pr.url}`} plain onPress={toggle(pr.url)}>
                <Text color={C.faint}>▸ </Text>
                <Text color={pr.tone === 'dim' ? C.dim : color}>{pr.tone === 'dim' ? '○' : '●'}</Text> <Text bold>{pr.name}</Text>
              </Button>
              <Text color={pr.tone === 'dim' ? C.faint : color}>{shorten(status, Math.max(8, W - pr.name.length - 8))}</Text>
            </Box>
            <Button key={`prtitle-${pr.url}`} plain dimColor={pr.tone === 'dim'} onPress={toggle(pr.url)}>
              {`    ${shorten(pr.title, W - 4)}`}
            </Button>
          </Box>
        )
      }
      const badge = pr.draft ? 'DRAFT' : 'READY'
      return (
        <Box flexDirection="column" borderStyle="round" borderColor={color} borderDimColor={pr.tone === 'dim'} paddingX={1} width={W}>
          <Box justifyContent="space-between">
            <Button key={`pr-${pr.url}`} plain onPress={toggle(pr.url)}>
              <Text color={C.faint}>▾ </Text>
              <Text bold color={color}>{pr.name}</Text>
              {pr.tag ? <Text color={C.purple}>{`  ${pr.tag}`}</Text> : pr.followed ? <Text color={C.purple}>  followed</Text> : null}
            </Button>
            <Button key={`badge-${pr.url}`} plain dimColor={pr.draft} onPress={press(() => act($, pr.draft ? 'pr.ready' : 'pr.draft', { url: pr.url, confirm: true }))}>
              <Text color={pr.draft ? C.dim : C.go} bold>{`[${badge}]`}</Text>
            </Button>
          </Box>
          <Button key={`title-${pr.url}`} plain onPress={press(() => open($, pr.url))}>
            <Text>{shorten(pr.title, inner)}</Text>
          </Button>
          {pr.labels.length ? <Text color={C.faint} wrap="truncate">{pr.labels.map(l => `#${l}`).join(' ')}</Text> : null}
          {pr.why ? <Text color={color} bold={pr.tone !== 'dim'} dimColor={pr.tone === 'dim'} wrap="truncate">{pr.why}</Text> : null}
          {railLine(pr)}
          {pr.checks.map((c, i) => checkLine(pr, c, i))}
          <Box columnGap={2} flexWrap="wrap">
            {pr.managed ? tick(`fix-${pr.url}`, 'auto-fix', pr.autofix, press(() => act($, 'pr.autofix', { url: pr.url, on: !pr.autofix }))) : null}
            {pr.managed ? tick(`review-${pr.url}`, 'auto-review', pr.autoreview, press(() => act($, 'pr.autoreview', { url: pr.url, on: !pr.autoreview }))) : null}
            {pr.automerge !== undefined
              ? tick(`merge-${pr.url}`, 'automerge', pr.automerge, press(() => act($, 'pr.automerge', { url: pr.url, on: !pr.automerge, confirm: true })))
              : null}
          </Box>
          <Box columnGap={1} flexWrap="wrap">
            {pr.fixLink ? <Button key={`fixit-${pr.url}`} variant="primary" label="fix it ↗" onPress={press(() => open($, pr.fixLink!))} /> : null}
            {pr.agentLink ? <Button key={`agent-${pr.url}`} label="its session ↗" onPress={press(() => open($, pr.agentLink!))} /> : null}
            <Button key={`open-${pr.url}`} label="GitHub ↗" onPress={press(() => open($, pr.url))} />
            {pr.followed ? <Button key={`unfollow-${pr.url}`} label="unfollow" onPress={press(() => act($, 'pr.unfollow', { url: pr.url }))} /> : null}
          </Box>
        </Box>
      )
    }
    const open_ = s.prs.filter(isOpen)
    const shut = s.prs.filter(p => !isOpen(p))
    // Everything one way: flip exactly the PRs whose default is the other way.
    const setAll = (open: boolean) => press(() => update($, view, x => ({ ...x, flipped: s.prs.filter(p => p.quiet === open).map(p => p.url) })))
    const prs = (
      <Box flexDirection="column" width={W}>
        <Box justifyContent="space-between">
          <Text bold>{`pull requests · ${s.prs.length} open${s.awaitingReview ? ` · ${s.awaitingReview} awaiting review` : ''}`}</Text>
          <Box columnGap={2}>
            {s.red ? <Text color={C.red} bold>{`${s.red} red`}</Text> : null}
            {s.prs.length ? (
              <Button key="prs-all" plain dimColor label={open_.length === 0 ? '▾ open all' : '▸ collapse all'} onPress={setAll(open_.length === 0)} />
            ) : null}
          </Box>
        </Box>
        {s.prs.length === 0 ? <Text color={C.faint}>{s.prLoading ? 'reading…' : 'no open PRs'}</Text> : null}
        {open_.map(prCard)}
        {shut.map(prCard)}
        {'Input' in els ? (
          <els.Input key="follow" placeholder="paste a PR or run link to follow" submitLabel="follow" onSubmit={text => void act($, 'pr.follow', { text })} />
        ) : null}
      </Box>
    )

    // ---- agents
    const agentGlyph = (a: Agent): [string, string] =>
      a.state === 'needs' ? ['!', C.amber] : a.state === 'working' ? ['◐', C.blue] : a.state === 'done' ? ['✓', C.go] : a.state === 'unread' ? ['●', C.text] : ['○', C.dim]
    const subLine = (a: Agent, sa: Subagent) => (
      <Box justifyContent="space-between">
        <Text wrap="truncate">
          <Text color={C.faint}>{`${' '.repeat(a.depth * 2)}  └ `}</Text>
          <Text color={sa.running ? C.blue : C.go}>{sa.running ? '◐ ' : '✓ '}</Text>
          <Text dimColor={!sa.running}>{shorten(sa.description || sa.kind, Math.max(10, inner - 22))}</Text>
          <Text color={C.faint}>{`  ${sa.kind}`}</Text>
        </Text>
        <Text color={sa.running ? C.blue : C.faint}>{sa.running ? 'working' : `${ago(sa.updatedAt, now)} ago`}</Text>
      </Box>
    )
    const noteLine = (a: Agent) => {
      const n = a.note
      if (!n) return null
      const pad = ' '.repeat(a.depth * 2 + 2)
      if (a.state === 'needs') return <Text color={C.amber} wrap="truncate">{`${pad}you: ${n.next || n.waiting || n.progress}`}</Text>
      if (n.waiting) return <Text color={C.blue} wrap="truncate">{`${pad}waiting: ${n.waiting}`}</Text>
      return n.progress || n.task ? <Text dimColor wrap="truncate">{`${pad}${n.progress || n.task}`}</Text> : null
    }
    const agentRow = (a: Agent) => {
      const [glyph, color] = agentGlyph(a)
      const label = (
        <Text>
          <Text color={C.faint}>{' '.repeat(a.depth * 2)}</Text>
          <Text color={color} bold>{`${glyph} `}</Text>
          <Text bold={a.state === 'needs' || a.state === 'unread'}>{shorten(a.label, Math.max(10, inner - 18))}</Text>
        </Text>
      )
      return (
        <Box flexDirection="column">
          <Box justifyContent="space-between">
            {a.link ? (
              <Button key={`agent-${a.key}`} plain onPress={press(() => open($, a.link!))}>
                <Text color={color} bold>{`${' '.repeat(a.depth * 2)}${glyph} `}</Text>
                <Text bold={a.state === 'needs' || a.state === 'unread'}>{shorten(a.label, Math.max(10, inner - 18))}</Text>
              </Button>
            ) : label}
            <Text>
              <Text color={C.faint}>{`${a.source} `}</Text>
              <Text color={color}>{a.status}</Text>
            </Text>
          </Box>
          {noteLine(a)}
          {a.subagents.map(sa => subLine(a, sa))}
        </Box>
      )
    }
    const unread = s.groups.flatMap(g => g.agents).filter(a => a.state === 'unread').length
    const agents = panel(
      s.needsYou ? C.amber : C.blue,
      `AGENTS · ${s.agentCount} running`,
      [s.working ? `${s.working} working` : '', s.needsYou ? `${s.needsYou} need you` : '', unread ? `${unread} ●` : ''].filter(Boolean).join(' · ') || null,
      [
        s.groups.length === 0 ? <Text color={C.faint}>no Claude Code sessions running</Text> : null,
        ...s.groups.map(g => (
          <Box flexDirection="column" marginTop={1}>
            <Text color={g.story ? C.purple : C.dim} bold>{g.label.toUpperCase()}</Text>
            {g.agents.map(agentRow)}
          </Box>
        )),
        s.earlier.length ? (
          <Box flexDirection="column" marginTop={1}>
            <Button key="earlier" plain dimColor onPress={press(() => update($, view, x => ({ ...x, showEarlier: !x.showEarlier })))}>
              {`${v.showEarlier ? '▾' : '▸'} EARLIER · ${s.earlier.length}`}
            </Button>
            {v.showEarlier
              ? s.earlier.map(n => (
                  <Box justifyContent="space-between">
                    <Text wrap="truncate" color={n.needsYou ? C.amber : undefined} dimColor={!n.needsYou}>{`${n.needsYou ? '!' : '○'} ${shorten(n.task || 'a session', inner - 8)}`}</Text>
                    <Text color={C.faint}>{ago(n.updatedAt, now)}</Text>
                  </Box>
                ))
              : null}
          </Box>
        ) : null,
      ],
      !s.needsYou && !s.working,
    )

    // ---- runs and watch
    const runRow = (r: Run) => {
      const [glyph, color] = RUN[r.state] ?? ['·', C.dim]
      const done = r.jobs.filter(j => j.state === 'success' || j.state === 'skipped').length
      return (
        <Box flexDirection="column">
          <Box justifyContent="space-between">
            <Button key={`run-${r.key}`} plain onPress={press(() => open($, r.url))}>
              <Text color={color} bold>{`${glyph} `}</Text>
              <Text bold>{r.repo}</Text> <Text>{shorten(r.name, Math.max(8, inner - r.repo.length - 18))}</Text>
            </Button>
            {r.state === 'running' || r.state === 'queued' ? timer(`run-clock-${r.key}`, r.startedAt, C.dim) : <Text color={color}>{r.state}</Text>}
          </Box>
          <Box justifyContent="space-between">
            <Text dimColor wrap="truncate">{`  ${shorten(r.title, inner - 16)}`}</Text>
            <Text color={C.faint}>{r.jobs.length ? `${done}/${r.jobs.length} jobs` : ''}</Text>
          </Box>
          {r.watched ? (
            <Box marginLeft={2}>
              <Button key={`unwatch-${r.key}`} plain dimColor label="stop watching" onPress={press(() => act($, 'run.unwatch', { repo: r.fullRepo, id: r.id }))} />
            </Box>
          ) : null}
        </Box>
      )
    }
    const runs = s.runsOn && s.runs.length
      ? panel(C.purple, `RUNS & WATCH · ${s.runs.length}`, null, s.runs.map(runRow), !s.runs.some(r => r.state === 'running' || r.state === 'queued'))
      : null

    // ---- log
    const time = (iso: string) => {
      const d = new Date(Date.parse(iso))
      return Number.isNaN(d.getTime()) ? '' : d.toTimeString().slice(0, 8)
    }
    const log = s.events.length ? (
      <Box flexDirection="column" borderStyle="round" borderColor={C.faint} paddingX={1} width={W}>
        <Button key="log" plain dimColor onPress={press(() => update($, view, x => ({ ...x, showLog: !x.showLog })))}>
          {`${v.showLog ? '▾' : '▸'} log`}
        </Button>
        {v.showLog
          ? s.events.slice(0, 6).map(ev => (
              <Box>
                <Box width={9} flexShrink={0}>
                  <Text color={C.faint}>{time(ev.at)}</Text>
                </Box>
                <Text wrap="truncate">{ev.text.replace(/^CAPCOM: /, '')}</Text>
              </Box>
            ))
          : null}
      </Box>
    ) : null

    const needs = s.prs.filter(p => p.tone === 'red' || p.tone === 'amber').length
    return (
      <Box flexDirection="column" rowGap={1}>
        <Box justifyContent="center">
          <Text bold>
            <Text>CAPCOM · </Text>
            <Text color={go ? C.go : C.red}>{go ? 'GO' : 'NO-GO'}</Text>
            <Text>{' · '}</Text>
            <Text color={needs ? C.amber : C.dim}>{`${needs} PR${needs === 1 ? '' : 'S'} TO FIX`}</Text>
            <Text>{' · '}</Text>
            <Text color={s.needsYou ? C.amber : C.dim}>{`${s.needsYou} NEED YOU`}</Text>
          </Text>
        </Box>
        {mission}
        {prs}
        {agents}
        {runs}
        {log}
      </Box>
    )
  })
}
