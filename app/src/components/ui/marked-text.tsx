import {
  useCallback,
  useMemo,
  type ChangeEvent,
  type CSSProperties,
  type ReactNode,
  type Ref,
  type TextareaHTMLAttributes,
} from 'react'
import { cn } from 'dowel-ui'

/*
 * Text with marks on it - read, or typed into.
 *
 * A textarea cannot colour a word. The way round it is older than React: draw
 * the same text twice, once as marked-up HTML underneath and once as the
 * textarea on top with its own text transparent, so the caret and the
 * selection are the browser's and the colours are ours. The two have to agree
 * on every metric - font, size, line height, padding, wrapping - or the marks
 * slide off the words they mark. So both take ONE class list, given by the
 * caller, and the textarea adds only what makes it invisible.
 *
 * The mirror is also what sizes the box. It sits in the flow and the textarea
 * is stretched over it, which means the field grows with its text without a
 * `scrollHeight` measurement and never scrolls inside itself - the box around
 * it scrolls, and the marks scroll with the words. A trailing space on the
 * last line keeps a final newline from being a line the mirror forgot.
 *
 * Two kinds of mark: a span of characters (a repeated word, a misspelling)
 * and a whole line (a line that is new since the other version). Lines are
 * drawn as blocks rather than separated by newlines so a line mark can paint
 * the full width; a block per line wraps exactly as the textarea's line does,
 * since it is the same text in the same width with the same font.
 */

/** A span of characters, as offsets into the whole text. */
export interface Mark {
  start: number
  end: number
  className?: string
  style?: CSSProperties
}

/** A whole line. */
export interface LineMark {
  /** Zero-based line index. */
  line: number
  className?: string
}

export interface MarkedLinesProps {
  /** The whole text. Lines are split on `\n`, and every mark's offsets count into this string. */
  text: string
  /** Spans of characters to mark, as offsets into `text`. Give them in order of `start` and do not overlap them. */
  marks?: readonly Mark[]
  /** Whole lines to mark, by zero-based index. A line mark paints the full width of the line. */
  lineMarks?: readonly LineMark[]
  /** Transparent text: the textarea above shows the letters. */
  ghost?: boolean
}

/** One line cut into runs, each run under at most one mark. */
function runs(line: string, offset: number, marks: readonly Mark[]): ReactNode[] {
  const out: ReactNode[] = []
  let at = 0
  for (const mark of marks) {
    const start = Math.max(mark.start - offset, 0)
    const end = Math.min(mark.end - offset, line.length)
    if (end <= at || start >= line.length) continue
    if (start > at) out.push(line.slice(at, start))
    out.push(
      <mark
        key={offset + start}
        // `<mark>` for the semantics; the browser's yellow is dropped so the
        // caller's class is the colour, in whichever tone the mark means.
        className={cn('rounded-sm bg-transparent text-inherit', mark.className)}
        style={mark.style}
      >
        {line.slice(start, end)}
      </mark>,
    )
    at = end
  }
  if (at < line.length) out.push(line.slice(at))
  return out
}

/** The marked-up text, line by line. The layer both the readable and the
 * typeable form are made of. */
export function MarkedLines({ text, marks = [], lineMarks = [], ghost = false }: MarkedLinesProps) {
  // Each line with where it starts in the text, so a mark given as an offset
  // into the whole can be cut to the line it falls on.
  const lines = useMemo(() => {
    const out: { line: string; start: number }[] = []
    let offset = 0
    for (const line of text.split('\n')) {
      out.push({ line, start: offset })
      offset += line.length + 1
    }
    return out
  }, [text])
  const byLine = useMemo(() => {
    const map = new Map<number, string | undefined>()
    for (const mark of lineMarks) map.set(mark.line, mark.className)
    return map
  }, [lineMarks])

  return (
    <>
      {lines.map(({ line, start }, index) => {
        // Only the marks that touch this line, in order; a text has few marks
        // and few lines, so a filter per line is cheaper than an index.
        const own = marks.filter((mark) => mark.end > start && mark.start < start + line.length)
        return (
          <div
            key={index}
            data-line={index}
            className={cn(
              'min-h-[1lh] whitespace-pre-wrap [overflow-wrap:anywhere]',
              ghost && 'text-transparent',
              byLine.get(index),
            )}
          >
            {/* A blank line still needs its height, and a trailing line needs
                a character to exist at all. */}
            {line === '' ? ' ' : runs(line, start, own)}
            {index === lines.length - 1 && ' '}
          </div>
        )
      })}
    </>
  )
}

export interface MarkedTextProps extends MarkedLinesProps {
  className?: string
}

/** Marked text to read. */
export function MarkedText({ className, ...lines }: MarkedTextProps) {
  return (
    <div className={cn('select-text', className)}>
      <MarkedLines {...lines} />
    </div>
  )
}

export interface MarkedTextareaProps
  extends Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, 'value' | 'onChange' | 'className'> {
  /** The ref of the textarea itself, for focus and selection. */
  ref?: Ref<HTMLTextAreaElement>
  /** The text, controlled. The field has no state of its own. */
  value: string
  /** The new value, not the event: the event is the textarea's business. */
  onChange: (value: string) => void
  /** Spans of characters to mark, as offsets into `value`. Give them in order of `start` and do not overlap them. */
  marks?: readonly Mark[]
  /** Whole lines to mark, by zero-based index. */
  lineMarks?: readonly LineMark[]
  /** The metrics both layers share: font, size, line height, padding. */
  className?: string
}

/** Marked text to type into. */
export function MarkedTextarea({
  value,
  onChange,
  marks,
  lineMarks,
  className,
  ref,
  ...props
}: MarkedTextareaProps) {
  const change = useCallback(
    (event: ChangeEvent<HTMLTextAreaElement>) => onChange(event.target.value),
    [onChange],
  )

  return (
    <div className="relative">
      {/* Both layers take the same size under a finger - 16px, for the reason
          `fieldClasses` gives - or the marks drift off the letters they mark. */}
      <div aria-hidden data-mirror className={cn('pointer-events-none pointer-coarse:text-lg', className)}>
        <MarkedLines text={value} marks={marks} lineMarks={lineMarks} ghost />
      </div>
      <textarea
        ref={ref}
        value={value}
        onChange={change}
        spellCheck={false}
        className={cn(
          // The same metrics, and nothing that would draw: the letters, the
          // caret and the selection are what this layer is for.
          'absolute inset-0 h-full w-full resize-none overflow-hidden pointer-coarse:text-lg text-text outline-none',
          '[overflow-wrap:anywhere] whitespace-pre-wrap',
          className,
          // After the caller's classes, on purpose. A background in the
          // metrics is meant for the box and lands on both layers; on this
          // one it would paint over every mark. The mirror keeps it, the
          // field never does.
          'bg-transparent',
        )}
        {...props}
      />
    </div>
  )
}
