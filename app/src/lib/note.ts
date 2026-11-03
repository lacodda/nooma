import { fromMarkdown } from 'mdast-util-from-markdown'
import { gfmFromMarkdown } from 'mdast-util-gfm'
import { gfm } from 'micromark-extension-gfm'
import type { Blockquote, Paragraph, Root } from 'mdast'

/*
 * A note, read as a note: the markdown of a passage parsed as CommonMark with
 * GitHub's tables, task lists, strikethrough and footnotes, and what Obsidian
 * adds to it read inside the text - links between notes, embeds, tags,
 * highlights, comments and callouts. Every node keeps where it is in the
 * passage, so the marks of a search land on the words they mark however the
 * note is drawn.
 */

/** A passage of a note, parsed. */
export function parseNote(text: string): Root {
  return fromMarkdown(text, { extensions: [gfm()], mdastExtensions: [gfmFromMarkdown()] })
}

/** A piece of a text node, as Obsidian reads it. Offsets count into the
 * node's value. */
export type Inline =
  | { kind: 'text'; start: number; end: number }
  /** `[[Note#Heading|shown]]`, or an embed `![[picture.png]]`. */
  | { kind: 'link'; start: number; end: number; target: string; shown: string; embed: boolean }
  | { kind: 'tag'; start: number; end: number }
  /** `==marked==`: `inner` is where the marked text is. */
  | { kind: 'highlight'; start: number; end: number; inner: [number, number] }
  /** `%%hidden%%`: not shown, as Obsidian does not show it. */
  | { kind: 'comment'; start: number; end: number }

/*
 * One pass over the text for all of them, in the order Obsidian gives them
 * way: a comment hides whatever is inside it, a link holds its own `#`.
 * A tag starts the text or follows a space or a bracket, and is not a number
 * alone - `#1` is not a tag.
 */
const OBSIDIAN = /%%[\s\S]*?%%|(!?)\[\[([^[\]|]*?)(?:\|([^[\]]*?))?\]\]|==(?=\S)([^=]*?\S)==|(^|[\s(])(#(?=[\p{N}_/-]*[\p{L}_/-])[\p{L}\p{N}_/-]+)/gu

/** What Obsidian shows for a link to a note: its name, and the heading in it
 * after a `>`. */
function shownTarget(target: string): string {
  const [note = '', ...rest] = target.split('#')
  const heading = rest.join('#')
  if (heading === '') return note
  return note === '' ? heading : `${note} > ${heading}`
}

/** A text node's value, cut into what Obsidian reads in it. */
export function inlinesOf(value: string): Inline[] {
  const out: Inline[] = []
  let at = 0
  for (const match of value.matchAll(OBSIDIAN)) {
    let start = match.index
    const whole = match[0]
    let inline: Inline
    if (whole.startsWith('%%')) {
      inline = { kind: 'comment', start, end: start + whole.length }
    } else if (match[2] !== undefined) {
      const target = match[2].trim()
      const alias = match[3]?.trim()
      inline = {
        kind: 'link',
        start,
        end: start + whole.length,
        target,
        shown: alias !== undefined && alias !== '' ? alias : shownTarget(target),
        embed: match[1] === '!',
      }
    } else if (match[4] !== undefined) {
      inline = { kind: 'highlight', start, end: start + whole.length, inner: [start + 2, start + whole.length - 2] }
    } else {
      // The space or bracket before a tag is the text's, not the tag's.
      start += (match[5] ?? '').length
      inline = { kind: 'tag', start, end: start + (match[6] ?? '').length }
    }
    if (start > at) out.push({ kind: 'text', start: at, end: start })
    out.push(inline)
    at = inline.end
  }
  if (at < value.length) out.push({ kind: 'text', start: at, end: value.length })
  return out
}

/** A callout: `> [!warning] Title`, its kind and the title it gives. */
export interface Callout {
  type: string
  title: string | null
  /** The quote's first paragraph without the line that made it a callout. */
  body: Paragraph | null
}

const CALLOUT = /^\[!([\w-]+)\][+-]?[ \t]*(.*)$/

/** The callout a quote is, if it is one. */
export function calloutOf(quote: Blockquote): Callout | null {
  const first = quote.children[0]
  if (first?.type !== 'paragraph') return null
  const opening = first.children[0]
  if (opening?.type !== 'text') return null
  const lineEnd = opening.value.indexOf('\n')
  const line = lineEnd < 0 ? opening.value : opening.value.slice(0, lineEnd)
  const match = CALLOUT.exec(line)
  if (match === null) return null
  const title = (match[2] ?? '').trim()
  // What follows the first line stays the paragraph it was, its place in
  // the passage moved past the line taken.
  let body: Paragraph | null = null
  if (lineEnd >= 0 || first.children.length > 1) {
    const rest = lineEnd < 0 ? '' : opening.value.slice(lineEnd + 1)
    const start = opening.position?.start
    const moved =
      start === undefined || start.offset === undefined
        ? undefined
        : { start: { ...start, offset: start.offset + (lineEnd + 1) }, end: opening.position!.end }
    const children = [...(rest === '' ? [] : [{ ...opening, value: rest, position: moved }]), ...first.children.slice(1)]
    body = children.length > 0 ? { ...first, children } : null
  }
  return { type: (match[1] ?? 'note').toLowerCase(), title: title === '' ? null : title, body }
}

/** The tone a callout of a kind is drawn in, as Obsidian groups its kinds. */
export function toneOf(type: string): 'info' | 'good' | 'warn' | 'bad' | 'neutral' {
  if (['tip', 'hint', 'important', 'success', 'check', 'done'].includes(type)) return 'good'
  if (['warning', 'caution', 'attention', 'question', 'help', 'faq'].includes(type)) return 'warn'
  if (['danger', 'error', 'failure', 'fail', 'missing', 'bug'].includes(type)) return 'bad'
  if (['example', 'quote', 'cite'].includes(type)) return 'neutral'
  return 'info'
}
