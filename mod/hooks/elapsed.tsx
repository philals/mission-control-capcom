// A running check's or run's clock, ticking on the surface's own frame clock so the pane need not redraw.
import type { ClientModule } from 'claude-code'

import { approx, clock } from './model'

type Props = { since: number; now: number; usual?: number; color: string }
type Ref = { base: number; ticks: number; lastNow: number }
type State = { ref: Ref }

const Elapsed: ClientModule<Props, State> = (props, surface) => {
  const { Text } = surface.elements
  const ref = surface.state?.ref ?? { base: 0, ticks: 0, lastNow: -1 }
  if (props.now !== ref.lastNow) {
    ref.lastNow = props.now
    ref.base = props.now - props.since
    ref.ticks = 0
  }
  if (surface.state === undefined) {
    surface.setState({ ref })
    surface.every(1000, () => {
      ref.ticks += 1
      surface.setState({ ref })
    })
  }
  const text = clock(ref.base + ref.ticks * 1000) + (props.usual ? ` of ~${approx(props.usual)}` : '')
  return <Text color={props.color}>{text}</Text>
}

export default Elapsed
