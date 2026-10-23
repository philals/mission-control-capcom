import { expect, mock, test } from 'claude-code/testing'
import type { TestBody } from 'claude-code/testing'

import { gauge, summarize, weekPct } from '../hooks/model'

const NOW = Date.parse('2026-10-23T01:00:00Z')

const STATE = {
  red: 1,
  awaitingReview: 1,
  prUpdated: '2026-10-23T00:59:50Z',
  prs: [
    {
      url: 'https://github.com/o/api/pull/1', repo: 'o/api', number: 1, title: 'Fix it', red: true, managed: true, autofix: true,
      fix: { why: 'CI failed', link: 'claude://code/new?prompt=x' }, counts: { failed: 1, running: 1 },
      rail: [{ state: 'done', text: 'ready' }, { state: 'wrong', text: 'CI' }],
      checks: [
        { label: 'build', state: 'passed' },
        { label: 'lint', state: 'running', startedAt: '2026-10-23T00:58:00Z', usual: 240 },
        { label: 'test', state: 'failed', url: 'https://github.com/o/api/actions/1' },
      ],
      automerge: { available: true, on: false },
    },
    { url: 'https://github.com/o/web/pull/2', repo: 'o/web', number: 2, title: 'Add it', waiting: 'reviewer', rail: [] },
    { url: 'https://github.com/o/web/pull/4', repo: 'o/web', number: 4, title: 'Quiet draft', draft: true, rail: [] },
  ],
  agents: {
    count: 2,
    working: 1,
    needsYou: 1,
    groups: [{ label: 'capcom', entries: [
      { pane: 'p1', label: 'T1 impl', state: 'needs', status: 'needs you', source: 'desktop', note: 0, deepLink: 'claude://code/continue?session=local_a', subagents: [] },
      { pane: 'p2', label: 'busy', state: 'working', status: 'working', source: 'cli', note: null, subagents: [{ id: 'a1', kind: 'Explore', description: 'Scan things', running: true }] },
    ] }],
    notes: [{ session: 's1', task: 'do T1', next: 'pick A or B', needsYou: true }],
    earlier: [],
  },
  runs: { on: true, items: [{ repo: 'o/api', id: 7, name: 'Deploy', title: 'release', state: 'running', url: 'https://github.com/o/api/actions/runs/7', watched: true, startedAt: '2026-10-23T00:55:00Z', jobs: [{ name: 'a', state: 'success' }, { name: 'b', state: 'running' }] }] },
  events: [{ at: '2026-10-23T00:50:00Z', text: 'CAPCOM: PR green, needs a reviewer: o/web#2' }],
  usage: { fiveHour: { pct: 72, resetsAt: '2026-10-23T02:30:00Z' }, sevenDay: { pct: 40 } },
}

const PANE = { plugin: 'capcom', component: 'Pane', requestId: 'capcom', props: { title: 'CAPCOM', isFocused: false, bodyColumns: 60, placement: 'dock', scroll: { offset: 0, bodyRows: 80 }, view: {} } } as const

test('the state is cut down to what the pane draws', () => {
  const s = summarize(STATE, NOW)
  expect(s.prs.map(p => [p.name, p.tone, p.quiet])).toEqual([
    ['api#1', 'red', false],
    ['web#2', 'green', false],
    ['web#4', 'dim', true],
  ])
  expect(s.prs[0]?.checks.map(c => c.label)).toEqual(['test', 'lint'])
  expect(s.prs[0]?.automerge).toBe(false)
  expect(s.groups[0]?.agents[0]?.note?.next).toBe('pick A or B')
  expect(s.groups[0]?.agents[1]?.subagents[0]?.kind).toBe('Explore')
  expect(s.runs[0]?.fullRepo).toBe('o/api')
  expect(gauge(72, 5)).toEqual({ on: '▮▮▮▮', off: '▯' })
  expect(weekPct(new Date(2026, 9, 24, 12))).toBe(100)
})

function world(on: Parameters<TestBody>[1], answer: () => string) {
  const posted: { path: string; body: any }[] = []
  on('command.register', () => ({ value: { command: 'capcom' } }))
  on('ui.open', () => ({ value: { isPlaced: true } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('http.fetch', (_, e) => {
    const path = new URL(e.url).pathname
    if (path === '/api/state') return { value: { status: 200, ok: true, headers: {}, text: answer() } }
    posted.push({ path, body: JSON.parse(e.init?.body ?? '{}') })
    return { value: { status: 200, ok: true, headers: {}, text: JSON.stringify({ ok: true, message: 'done it' }) } }
  })
  return posted
}

test('the console draws on every surface, and links open through capcom web', async ($, on) => {
  const clock = mock.clock(on, { now: NOW })
  const posted = world(on, () => JSON.stringify(STATE))
  await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })
  await clock.settle()

  for (const surface of ['terminal', 'desktop', 'vscode', 'mobile'] as const) {
    const ui = await $.ui.mount({ ...PANE, surface })
    for (const words of [/NO-GO · 1 red/, /72%/, /pick A or B/, /Scan things/, /Deploy/, /needs a reviewer/, /CI failed/]) {
      expect(await ui.find({ type: 'Text', text: words })).toBeDefined()
    }
    await ui.press({ key: 'title-https://github.com/o/api/pull/1' })
    await ui.press({ key: 'agent-p1' })
    await ui.unmount()
  }
  await clock.settle()
  expect(posted.filter(p => p.path === '/open').slice(0, 2).map(p => p.body.url)).toEqual([
    'https://github.com/o/api/pull/1',
    'claude://code/continue?session=local_a',
  ])
})

test('marking ready asks first, then sends the confirmed action', async ($, on) => {
  const clock = mock.clock(on, { now: NOW })
  const draft = { ...STATE, prs: [{ ...STATE.prs[0], draft: true }] }
  const posted = world(on, () => JSON.stringify(draft))
  await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })
  await clock.settle()

  const ui = await $.ui.mount({ ...PANE, surface: 'desktop' })
  await ui.press({ key: 'badge-https://github.com/o/api/pull/1' })
  expect(posted.filter(p => p.path === '/api/do')).toEqual([])
  expect(await ui.find({ type: 'Text', text: /Mark ready for review\?/ })).toBeDefined()
  await ui.press({ key: 'yes-https://github.com/o/api/pull/1' })
  await clock.settle()
  expect(posted.filter(p => p.path === '/api/do').map(p => p.body)).toEqual([{ action: 'pr.ready', url: 'https://github.com/o/api/pull/1', confirm: true }])
  expect(await ui.find({ type: 'Text', text: /done it/ })).toBeDefined()
  await ui.unmount()
})

test('a capcom web that does not answer is said', async ($, on) => {
  const clock = mock.clock(on, { now: NOW })
  world(on, () => {
    throw new Error('connection refused')
  })
  await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })
  await clock.settle()
  const ui = await $.ui.mount({ ...PANE, surface: 'desktop' })
  expect(await ui.find({ type: 'Text', text: /not answering/ })).toBeDefined()
  await ui.unmount()
})
