import type { ReactNode } from 'react'
import type { Blockquote, List, Nodes, Paragraph, PhrasingContent, RootContent, Table } from 'mdast'
import { cn } from '@/lib/utils'
import { type Mark, type MarkKind, placesOf, runsAt } from '@/lib/marks'
import { calloutOf, inlinesOf, parseNote, toneOf } from '@/lib/note'

/*
 * A passage of a note drawn as the note reads, not as its markdown is typed:
 * headings, lists and tasks, quotes and callouts, tables, code, links between
 * notes. It is a reading of the note, and nothing in it acts: a link is drawn
 * as one and goes nowhere - the preview is not a browser, and a click that
 * left the window for a web page would be a click that sent something out.
 * Pictures are named where they stand, not loaded.
 *
 * Marks count in the passage's own text, and every node knows where in it
 * it stands, so a word of the query is marked in a heading, a list item, a
 * quote or a table cell alike. A node whose letters cannot be placed in the
 * passage - an entity stands for one - is not cut by marks at all.
 */

export const MARK_CLASSES: Record<MarkKind, string> = {
  word: 'border-b border-info bg-info-soft px-px text-text',
  closest: 'rounded-xs bg-accent-soft px-px text-text',
}

/** Text with the marks that fall on the places its letters stand at. */
function Placed({ text, places, marks }: { text: string; places: readonly number[]; marks: readonly Mark[] }) {
  return (
    <>
      {runsAt(text, places, marks).map((run, index) =>
        run.mark === null ? (
          <span key={index}>{run.text}</span>
        ) : (
          <mark key={index} className={MARK_CLASSES[run.mark]}>
            {run.text}
          </mark>
        ),
      )}
    </>
  )
}

interface Context {
  source: string
  marks: readonly Mark[]
}

export function Note({ text, marks, className }: { text: string; marks: readonly Mark[]; className?: string }) {
  const tree = parseNote(text)
  const context = { source: text, marks }
  return <div className={cn('text-sm leading-relaxed [overflow-wrap:anywhere]', className)}>{blocks(tree.children, context)}</div>
}

/** Where the letters of a node's value stand in the passage. */
function places(node: Nodes & { value: string }, context: Context): number[] | null {
  const start = node.position?.start.offset
  const end = node.position?.end.offset
  if (start === undefined || end === undefined || context.marks.length === 0) return null
  return placesOf(node.value, context.source, start, end)
}

function markedValue(node: Nodes & { value: string }, context: Context): ReactNode {
  const at = places(node, context)
  return at === null ? node.value : <Placed text={node.value} places={at} marks={context.marks} />
}

/** A text node, with what Obsidian reads in it. */
function text(value: string, at: number[] | null, context: Context): ReactNode[] {
  const marked = (from: number, to: number) =>
    at === null ? value.slice(from, to) : <Placed text={value.slice(from, to)} places={at.slice(from, to)} marks={context.marks} />
  const hit = (from: number, to: number) =>
    at !== null && at.slice(from, to).some((place) => context.marks.some((mark) => place >= mark.start && place < mark.end))
  return inlinesOf(value).map((inline, index) => {
    switch (inline.kind) {
      case 'text':
        return <span key={index}>{marked(inline.start, inline.end)}</span>
      case 'comment':
        return null
      case 'highlight':
        return (
          <span key={index} className="rounded-xs bg-warn-soft px-px">
            {marked(inline.inner[0], inline.inner[1])}
          </span>
        )
      case 'tag':
        return (
          <span key={index} className={cn('text-accent', hit(inline.start, inline.end) && MARK_CLASSES.word)}>
            {value.slice(inline.start, inline.end)}
          </span>
        )
      case 'link':
        return (
          <span
            key={index}
            title={inline.target}
            className={cn(
              'text-accent underline decoration-dotted underline-offset-2',
              inline.embed && 'rounded-xs border border-line px-1 no-underline',
              hit(inline.start, inline.end) && MARK_CLASSES.word,
            )}
          >
            {inline.shown}
          </span>
        )
    }
  })
}

function phrasing(nodes: readonly PhrasingContent[], context: Context): ReactNode[] {
  return nodes.map((node, index) => {
    switch (node.type) {
      case 'text':
        return <span key={index}>{text(node.value, places(node, context), context)}</span>
      case 'emphasis':
        return <em key={index}>{phrasing(node.children, context)}</em>
      case 'strong':
        return (
          <strong key={index} className="font-semibold">
            {phrasing(node.children, context)}
          </strong>
        )
      case 'delete':
        return (
          <del key={index} className="text-dim">
            {phrasing(node.children, context)}
          </del>
        )
      case 'inlineCode':
        return (
          <code key={index} className="rounded-xs bg-soft px-1 font-mono text-[0.9em]">
            {markedValue(node, context)}
          </code>
        )
      case 'link':
      case 'linkReference':
        return (
          <span key={index} title={node.type === 'link' ? node.url : undefined} className="text-accent underline decoration-dotted underline-offset-2">
            {phrasing(node.children, context)}
          </span>
        )
      case 'image':
      case 'imageReference':
        return (
          <span key={index} title={node.type === 'image' ? node.url : undefined} className="rounded-xs border border-line px-1 text-dim">
            {node.alt !== null && node.alt !== undefined && node.alt !== '' ? node.alt : node.type === 'image' ? node.url : '…'}
          </span>
        )
      case 'break':
        return <br key={index} />
      case 'footnoteReference':
        return (
          <sup key={index} className="text-faint">
            [{node.label ?? node.identifier}]
          </sup>
        )
      case 'html':
        return (
          <span key={index} className="font-mono text-xs text-faint">
            {node.value}
          </span>
        )
      default:
        return null
    }
  })
}

function paragraph(node: Paragraph, context: Context, key: number, tight = false): ReactNode {
  return (
    <p key={key} className={tight ? 'my-0' : 'my-2 first:mt-0 last:mb-0'}>
      {phrasing(node.children, context)}
    </p>
  )
}

const HEADING_CLASSES = ['', 'text-base font-semibold', 'text-sm font-semibold', 'text-sm font-semibold text-dim']

const TONE_CLASSES: Record<ReturnType<typeof toneOf>, string> = {
  info: 'border-info bg-info-soft',
  good: 'border-good bg-good-soft',
  warn: 'border-warn bg-warn-soft',
  bad: 'border-bad bg-bad-soft',
  neutral: 'border-line bg-soft',
}

function quote(node: Blockquote, context: Context, key: number): ReactNode {
  const callout = calloutOf(node)
  if (callout === null) {
    return (
      <blockquote key={key} className="my-2 border-l-2 border-line pl-3 text-dim first:mt-0 last:mb-0">
        {blocks(node.children, context)}
      </blockquote>
    )
  }
  const title = callout.title ?? callout.type.charAt(0).toUpperCase() + callout.type.slice(1)
  return (
    <div key={key} data-callout={callout.type} className={cn('my-2 rounded-md border-l-2 px-3 py-2 first:mt-0 last:mb-0', TONE_CLASSES[toneOf(callout.type)])}>
      <p className="my-0 font-semibold">{title}</p>
      {callout.body !== null && paragraph(callout.body, context, -1)}
      {blocks(node.children.slice(1), context)}
    </div>
  )
}

function list(node: List, context: Context, key: number): ReactNode {
  const items = node.children.map((item, index) => {
    const content = item.children.map((child, at) => (child.type === 'paragraph' ? paragraph(child, context, at, !node.spread) : block(child, context, at)))
    if (item.checked === null || item.checked === undefined) return <li key={index}>{content}</li>
    return (
      <li key={index} className="-ml-5 flex list-none items-baseline gap-2">
        <input type="checkbox" checked={item.checked} disabled readOnly className="translate-y-0.5 accent-accent" />
        <div className={cn('min-w-0', item.checked && 'text-dim line-through')}>{content}</div>
      </li>
    )
  })
  const classes = 'my-2 pl-5 first:mt-0 last:mb-0'
  return node.ordered ? (
    <ol key={key} start={node.start ?? undefined} className={cn(classes, 'list-decimal')}>
      {items}
    </ol>
  ) : (
    <ul key={key} className={cn(classes, 'list-disc')}>
      {items}
    </ul>
  )
}

function table(node: Table, context: Context, key: number): ReactNode {
  const [head, ...rows] = node.children
  const align = (index: number) => {
    const side = node.align?.[index]
    return side === 'center' ? 'text-center' : side === 'right' ? 'text-right' : 'text-left'
  }
  return (
    <div key={key} className="my-2 overflow-x-auto first:mt-0 last:mb-0">
      <table className="border-collapse text-xs">
        {head !== undefined && (
          <thead>
            <tr>
              {head.children.map((cell, index) => (
                <th key={index} className={cn('border border-line px-2 py-1 font-semibold', align(index))}>
                  {phrasing(cell.children, context)}
                </th>
              ))}
            </tr>
          </thead>
        )}
        <tbody>
          {rows.map((row, at) => (
            <tr key={at}>
              {row.children.map((cell, index) => (
                <td key={index} className={cn('border border-line px-2 py-1 align-top', align(index))}>
                  {phrasing(cell.children, context)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function block(node: RootContent, context: Context, key: number): ReactNode {
  switch (node.type) {
    case 'paragraph':
      return paragraph(node, context, key)
    case 'heading':
      return (
        <p key={key} role="heading" aria-level={Math.min(node.depth + 2, 6)} className={cn('mt-3 mb-1 first:mt-0', HEADING_CLASSES[Math.min(node.depth, 3)])}>
          {phrasing(node.children, context)}
        </p>
      )
    case 'blockquote':
      return quote(node, context, key)
    case 'list':
      return list(node, context, key)
    case 'table':
      return table(node, context, key)
    case 'code':
      return (
        <pre key={key} className="my-2 overflow-x-auto rounded-md bg-soft p-2 font-mono text-xs leading-normal first:mt-0 last:mb-0">
          <code>{markedValue(node, context)}</code>
        </pre>
      )
    case 'thematicBreak':
      return <hr key={key} className="my-3 border-line" />
    case 'html':
      return (
        <p key={key} className="my-2 font-mono text-xs whitespace-pre-wrap text-faint">
          {node.value}
        </p>
      )
    case 'footnoteDefinition':
      return (
        <div key={key} className="my-1 flex gap-1 text-xs text-dim">
          <span className="text-faint">[{node.label ?? node.identifier}]</span>
          <div className="min-w-0">{blocks(node.children, context)}</div>
        </div>
      )
    default:
      // A link's definition is where a reference points, not text.
      return null
  }
}

function blocks(nodes: readonly RootContent[], context: Context): ReactNode[] {
  return nodes.map((node, index) => block(node, context, index))
}
