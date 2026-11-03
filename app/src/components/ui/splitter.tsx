import {
  Children,
  createContext,
  isValidElement,
  useContext,
  useId,
  useRef,
  useState,
  type HTMLAttributes,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type RefObject,
} from 'react'
import { cva } from 'class-variance-authority'
import { cn } from 'dowel-ui'

/*
 * Panes side by side, or one above the other, with a handle between each two
 * that moves the boundary.
 *
 * Three parts - `Splitter`, `SplitterPane`, `SplitterHandle` - rather than a
 * component with a `first` and a `second`. A two-pane prop shape is the one
 * every product outgrows on the day it wants a third pane, and the rewrite is
 * every call site. Here a third pane is two more lines of markup. Each handle
 * moves only the boundary it sits on: the pane before it and the pane after
 * it trade size, and the others stay where they are, which is what a reader
 * dragging one edge expects.
 *
 * Sizes are percentages of the splitter, not pixels, so a window resized to
 * half its width keeps the sidebar at the same share of it instead of eating
 * the editor. A pane is laid out with its percentage as its flex grow and a
 * zero basis, which is what lets the one-pixel handles sit between them
 * without the percentages adding up to more than the box.
 *
 * The limits live on the panes (`min`, `max`, `collapsible`), because that is
 * where a reader of the markup looks for them, and the splitter reads them off
 * its children when it clamps. The price is that panes and handles must be
 * direct children - not wrapped in a fragment or a component of the product's
 * own - since that is how each learns its place in the row. The arithmetic is
 * exported as plain functions for the same reason `virtual-list`'s is: jsdom
 * has no layout, and a rule that can only be checked in a browser is a rule
 * that goes unchecked.
 *
 * The component keeps the sizes and never stores them. `onSizesChange` fires
 * when a change settles - a drag let go, a key pressed - and not on every
 * move, so a product persisting there writes once per gesture rather than
 * sixty times a second. What it saved comes back as `defaultSizes`.
 *
 * The handle is the ARIA window splitter: a focusable `separator` carrying the
 * size of the pane before it as its value, so a screen reader says how much
 * room that pane has and the arrows change it. Enter collapses a pane marked
 * `collapsible` and restores it to the size it had. The visible line is a
 * pixel; the hit area is `target-min`, the theme's named way of growing a
 * target past the floor without growing what is drawn. There is no drag by
 * HTML5 drag-and-drop, for the reason `column-resize-handle` gives: a desktop
 * shell that takes file drops never lets `dragstart` reach the page. Pointer
 * capture keeps the drag alive when the pointer runs ahead of the handle.
 */

/** What a pane allows, in percent of the splitter. */
export interface PaneLimits {
  /** Smallest size, 0 by default. */
  min?: number
  /** Largest size, 100 by default. */
  max?: number
  /** Whether the pane can shrink past `min` to `collapsedSize`. */
  collapsible?: boolean
  /** Its size when collapsed: 0 by default, a rail's width for a pane that
   * keeps its icons. */
  collapsedSize?: number
}

const floorOf = (limits: PaneLimits) => (limits.collapsible ? (limits.collapsedSize ?? 0) : (limits.min ?? 0))

/** The range the pane before handle `index` can take, given what both panes
 * beside the handle allow. */
export function boundsOf(sizes: number[], index: number, limits: PaneLimits[]): [number, number] {
  const before = limits[index] ?? {}
  const after = limits[index + 1] ?? {}
  const pair = sizes[index]! + sizes[index + 1]!
  return [Math.max(floorOf(before), pair - (after.max ?? 100)), Math.min(before.max ?? 100, pair - floorOf(after))]
}

/* A collapsible pane below its minimum is either collapsed or at its minimum,
 * never between. Dragging, it goes to whichever is nearer; by keyboard the
 * direction decides, or one press past the minimum would land on the minimum
 * again and the pane could never be collapsed by arrows. */
function snap(size: number, limits: PaneLimits, direction: number): number {
  const min = limits.min ?? 0
  const collapsed = limits.collapsedSize ?? 0
  if (!limits.collapsible || size >= min) return size
  return (direction === 0 ? size < (min + collapsed) / 2 : direction < 0) ? collapsed : min
}

/** New sizes after handle `index` asks for the pane before it to be `target`.
 * `direction` is the sign of a key press, or 0 for a drag. */
export function moveBoundary(
  sizes: number[],
  index: number,
  target: number,
  limits: PaneLimits[],
  direction = 0,
): number[] {
  const pair = sizes[index]! + sizes[index + 1]!
  const [low, high] = boundsOf(sizes, index, limits)
  let size = snap(target, limits[index] ?? {}, direction)
  size = pair - snap(pair - size, limits[index + 1] ?? {}, -direction)
  // Two decimals: finer than a pixel on any screen, and a persisted value
  // that reads as a number rather than as float noise.
  size = Math.round(Math.min(high, Math.max(low, size)) * 100) / 100
  const next = [...sizes]
  next[index] = size
  next[index + 1] = Math.round((pair - size) * 100) / 100
  return next
}

interface SplitterState {
  orientation: 'horizontal' | 'vertical'
  sizes: number[]
  limits: PaneLimits[]
  step: number
  paneId: (index: number) => string
  root: RefObject<HTMLDivElement | null>
  move: (index: number, target: number, direction: number, done: boolean) => void
}

const State = createContext<SplitterState | null>(null)
const Place = createContext(0)

function useSplitter() {
  const state = useContext(State)
  if (state === null) throw new Error('SplitterPane and SplitterHandle belong inside a Splitter')
  return state
}

export interface SplitterProps extends Omit<HTMLAttributes<HTMLDivElement>, 'children'> {
  /** How the panes are laid out: `horizontal` is side by side, with upright
   * handles between them; `vertical` stacks them. */
  orientation?: 'horizontal' | 'vertical'
  /** One percentage per pane, summing to 100 - what was saved from
   * `onSizesChange`, or the product's opening layout. Equal shares without. */
  defaultSizes?: number[]
  /** Told the sizes when a change settles: persist them here. */
  onSizesChange?: (sizes: number[]) => void
  /** Percent moved by an arrow key; Shift moves five of these. */
  step?: number
  children: ReactNode
}

export function Splitter({
  orientation = 'horizontal',
  defaultSizes,
  onSizesChange,
  step = 2,
  className,
  children,
  ...props
}: SplitterProps) {
  const id = useId()
  const root = useRef<HTMLDivElement>(null)
  const limits: PaneLimits[] = []
  const placed = Children.toArray(children)
    .filter(isValidElement)
    .map((child) => {
      if (child.type === SplitterPane) limits.push(child.props as PaneLimits)
      // Counted after the push, so a pane's place is its own index and a
      // handle's is the index of the pane before it.
      return (
        <Place.Provider key={child.key} value={limits.length - 1}>
          {child}
        </Place.Provider>
      )
    })
  const even = () => limits.map(() => 100 / limits.length)
  const [kept, setKept] = useState<number[]>(() => defaultSizes ?? even())
  // A pane added or removed invalidates every share: start again from equal.
  const sizes = kept.length === limits.length ? kept : even()

  const move = (index: number, target: number, direction: number, done: boolean) => {
    const next = moveBoundary(sizes, index, target, limits, direction)
    setKept(next)
    if (done) onSizesChange?.(next)
  }

  return (
    <State.Provider
      value={{ orientation, sizes, limits, step, root, move, paneId: (index) => `${id}-pane-${index}` }}
    >
      <div
        ref={root}
        data-orientation={orientation}
        className={cn('flex size-full min-h-0 min-w-0', orientation === 'vertical' && 'flex-col', className)}
        {...props}
      >
        {placed}
      </div>
    </State.Provider>
  )
}

export interface SplitterPaneProps extends Omit<HTMLAttributes<HTMLDivElement>, 'id'>, PaneLimits {}

// `min` and `max` are taken out only so they do not land on the `div`: the
// splitter reads them off the element, and the pane has no use for them.
// eslint-disable-next-line @typescript-eslint/no-unused-vars
export function SplitterPane({ min, max, collapsible, collapsedSize = 0, className, style, ...props }: SplitterPaneProps) {
  const { sizes, paneId } = useSplitter()
  const index = useContext(Place)
  const size = sizes[index] ?? 0
  return (
    <div
      id={paneId(index)}
      data-collapsed={collapsible && size <= collapsedSize ? '' : undefined}
      // A pane collapsed to nothing still holds its content, and without
      // `inert` a Tab walks into controls nobody can see.
      inert={size === 0}
      style={{ flex: `${size} 1 0px`, ...style }}
      className={cn('min-h-0 min-w-0 overflow-hidden', className)}
      {...props}
    />
  )
}

export const splitterHandleVariants = cva(
  cn(
    'z-10 shrink-0 touch-none select-none bg-line target-min outline-none',
    '[transition:background-color_var(--duration-quick)_var(--ease-out)]',
    'hover:bg-accent data-[dragging]:bg-accent',
    'focus-visible:bg-accent focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent',
  ),
  {
    variants: {
      /* The splitter's orientation, so the handle's line runs across it. */
      orientation: {
        horizontal: 'w-px cursor-col-resize',
        vertical: 'h-px cursor-row-resize',
      },
    },
  },
)

export interface SplitterHandleProps extends Omit<HTMLAttributes<HTMLDivElement>, 'children'> {
  /** Names what the handle resizes: "Resize the sidebar". No default - it is
   * the product's word. */
  label: string
}

export function SplitterHandle({ label, className, ...props }: SplitterHandleProps) {
  const { orientation, sizes, limits, step, paneId, root, move } = useSplitter()
  const index = useContext(Place)
  const origin = useRef<{ at: number; size: number; span: number } | null>(null)
  // The size a collapsed pane goes back to on the next Enter.
  const restore = useRef<number | undefined>(undefined)
  const [dragging, setDragging] = useState(false)
  const across = orientation === 'horizontal'
  const size = sizes[index] ?? 0
  const [low, high] = boundsOf(sizes, index, limits)

  const drag = (event: ReactPointerEvent<HTMLElement>, done: boolean) => {
    const from = origin.current
    if (from === null) return
    const travelled = (across ? event.clientX : event.clientY) - from.at
    move(index, from.size + (travelled / from.span) * 100, 0, done)
  }

  const begin = (event: ReactPointerEvent<HTMLElement>) => {
    // The primary button only: a right-click is the context menu's.
    if (event.button !== 0 || root.current === null) return
    const box = root.current.getBoundingClientRect()
    const span = across ? box.width : box.height
    if (span === 0) return
    event.preventDefault()
    origin.current = { at: across ? event.clientX : event.clientY, size, span }
    event.currentTarget.setPointerCapture(event.pointerId)
    setDragging(true)
  }

  const end = (event: ReactPointerEvent<HTMLElement>) => {
    drag(event, true)
    origin.current = null
    setDragging(false)
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
  }

  // The pane before the handle collapses if it can, else the one after it: a
  // sidebar on the right is the second pane of its handle.
  const toggled = (): number | undefined => {
    const pane = limits[index]?.collapsible ? index : limits[index + 1]?.collapsible ? index + 1 : -1
    if (pane < 0) return undefined
    const own = sizes[pane]!
    const collapsed = limits[pane]!.collapsedSize ?? 0
    let next = collapsed
    if (own <= collapsed) next = restore.current ?? Math.max(limits[pane]!.min ?? 0, 100 / sizes.length)
    else restore.current = own
    return pane === index ? next : size + sizes[index + 1]! - next
  }

  const press = (event: KeyboardEvent<HTMLElement>) => {
    const by = event.shiftKey ? step * 5 : step
    const target = {
      [across ? 'ArrowLeft' : 'ArrowUp']: size - by,
      [across ? 'ArrowRight' : 'ArrowDown']: size + by,
      Home: low,
      End: high,
    }[event.key] ?? (event.key === 'Enter' ? toggled() : undefined)
    if (target === undefined) return
    event.preventDefault()
    move(index, target, Math.sign(target - size), true)
  }

  return (
    <div
      role="separator"
      tabIndex={0}
      aria-label={label}
      aria-controls={paneId(index)}
      // The line runs across the layout: panes side by side are split by an
      // upright separator.
      aria-orientation={across ? 'vertical' : 'horizontal'}
      aria-valuenow={Math.round(size)}
      aria-valuemin={Math.round(low)}
      aria-valuemax={Math.round(high)}
      data-dragging={dragging ? '' : undefined}
      onPointerDown={begin}
      onPointerMove={(event) => drag(event, false)}
      onPointerUp={end}
      onPointerCancel={end}
      onKeyDown={press}
      className={cn(splitterHandleVariants({ orientation }), className)}
      {...props}
    />
  )
}
