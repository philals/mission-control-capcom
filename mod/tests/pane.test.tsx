import { expect, mock, test } from 'claude-code/testing'

import { summarize } from '../hooks/register'

const NOW = Date.parse('2026-10-23T01:00:00Z')

const STATE = {
  red: 1,
  awaitingReview: 1,
  prs: [
    { url: 'https://github.com/o/api/pull/1', repo: 'o/api', number: 1, title: 'Fix it', red: true, fix: { why: 'CI failed', link: 'claude://code/new?prompt=x' } },
    { url: 'https://github.com/o/web/pull/2', repo: 'o/web', number: 2, title: 'Add it', waiting: 'reviewer' },
    { url: 'https://github.com/o/web/pull/3', repo: 'o/web', number: 3, title: 'Behind', fix: { why: 'behind', onlyBehind: true }, autofix: true },
    { url: 'https://github.com/o/web/pull/4', repo: 'o/web', number: 4, title: 'Quiet draft', draft: true },
  ],
  agents: {
    working: 2,
    groups: [{ entries: [
      { pane: 'p1', label: 'T1 impl', state: 'needs', note: 0, deepLink: 'claude://code/continue?session=local_a' },
      { pane: 'p2', label: 'busy', state: 'working', note: null },
    ] }],
    notes: [{ session: 's1', next: 'pick A or B', needsYou: true }],
    earlier: [],
  },
  usage: { fiveHour: { pct: 72, resetsAt: '2026-10-23T02:30:00Z' }, sevenDay: { pct: 40 } },
}

test('only the PRs and agents that need you are kept', () => {
  const s = summarize(STATE, NOW)
  expect(s.prs.map(p => [p.name, p.why, p.tone])).toEqual([
    ['api#1', 'CI failed', 'red'],
    ['web#2', 'awaiting review', 'green'],
  ])
  expect(s.prs[0]?.fix).toBe('claude://code/new?prompt=x')
  expect(s.agents).toEqual([{ key: 'p1', label: 'T1 impl', ask: 'pick A or B', link: 'claude://code/continue?session=local_a' }])
  expect(s.fiveHour?.pct).toBe(72)
  expect(s.prCount).toBe(4)
})

test('the pane draws the console on every surface and opens links through capcom web', async ($, on) => {
  const clock = mock.clock(on, { now: NOW })
  const posted: string[] = []
  on('command.register', () => ({ value: { command: 'capcom' } }))
  on('ui.open', () => ({ value: { isPlaced: true } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('http.fetch', (_, e) => {
    if (e.url.endsWith('/open')) {
      posted.push(JSON.parse(e.init?.body ?? '{}').url)
      return { value: { status: 204, ok: true, headers: {}, text: '' } }
    }
    return { value: { status: 200, ok: true, headers: {}, text: JSON.stringify(STATE) } }
  })

  await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })
  await clock.settle()

  for (const surface of ['terminal', 'desktop', 'vscode', 'mobile'] as const) {
    const ui = await $.ui.mount({
      plugin: 'capcom', component: 'Pane', requestId: 'capcom', surface,
      props: { title: 'CAPCOM', isFocused: false, bodyColumns: 50, placement: 'dock', scroll: { offset: 0, bodyRows: 30 }, view: {} },
    })
    expect((await ui.find({ type: 'Text', text: /NO-GO · 1 red/ }))).toBeDefined()
    expect((await ui.find({ type: 'Text', text: /5H 72%/ }))).toBeDefined()
    expect((await ui.find({ type: 'Text', text: /pick A or B/ }))).toBeDefined()
    await ui.press({ key: 'https://github.com/o/api/pull/1' })
    await ui.press({ key: 'p1' })
    await ui.unmount()
  }
  await clock.settle()
  expect(posted.slice(0, 2)).toEqual(['https://github.com/o/api/pull/1', 'claude://code/continue?session=local_a'])
})

test('a capcom web that does not answer is said', async ($, on) => {
  const clock = mock.clock(on, { now: NOW })
  on('command.register', () => ({ value: { command: 'capcom' } }))
  on('ui.open', () => ({ value: { isPlaced: true } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))
  on('http.fetch', () => { throw new Error('connection refused') })

  await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })
  await clock.settle()
  const ui = await $.ui.mount({
    plugin: 'capcom', component: 'Pane', requestId: 'capcom', surface: 'desktop',
    props: { title: 'CAPCOM', isFocused: false, bodyColumns: 50, placement: 'dock', scroll: { offset: 0, bodyRows: 30 }, view: {} },
  })
  expect((await ui.find({ type: 'Text', text: /not answering/ }))).toBeDefined()
  await ui.unmount()
})
