/*
 * Marks on a whole passage: the query's words, which the core reports as
 * byte offsets into UTF-8, and the sentence that answered by meaning, which
 * it reports inside a fragment cut from the passage. Both become offsets
 * into the passage as JavaScript indexes it - UTF-16 units - once, here, so
 * every renderer below counts in one unit.
 */

/** What a mark marks: a word of the query, or the sentence closest in
 * meaning to it. */
export type MarkKind = 'word' | 'closest'

/** A mark, as UTF-16 offsets into the text it is on. */
export interface Mark {
  start: number
  end: number
  kind: MarkKind
}

/** UTF-8 byte offsets into a text, as UTF-16 offsets into the same text. A
 * range that does not fall on characters' edges is dropped. */
export function utf16Ranges(text: string, ranges: readonly (readonly [number, number])[]): [number, number][] {
  const wanted = new Set<number>()
  for (const [start, end] of ranges) {
    wanted.add(start)
    wanted.add(end)
  }
  const at = new Map<number, number>()
  let bytes = 0
  for (let index = 0; index < text.length; ) {
    if (wanted.has(bytes)) at.set(bytes, index)
    const code = text.codePointAt(index) ?? 0
    bytes += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4
    index += code > 0xffff ? 2 : 1
  }
  if (wanted.has(bytes)) at.set(bytes, text.length)
  const out: [number, number][] = []
  for (const [start, end] of ranges) {
    const from = at.get(start)
    const to = at.get(end)
    if (from !== undefined && to !== undefined && from < to) out.push([from, to])
  }
  return out.sort((a, b) => a[0] - b[0])
}

/** Where the sentence a fragment marks is in the passage the fragment was
 * cut from; `null` when the passage does not hold it - it changed since. */
export function sentenceIn(text: string, fragment: string, closest: readonly [number, number]): [number, number] | null {
  const bytes = new TextEncoder().encode(fragment)
  const sentence = new TextDecoder().decode(bytes.subarray(closest[0], closest[1])).trim()
  if (sentence === '') return null
  const at = text.indexOf(sentence)
  return at < 0 ? null : [at, at + sentence.length]
}

/** Marks in order, none overlapping the one before it. */
export function ordered(marks: readonly Mark[]): Mark[] {
  const out: Mark[] = []
  for (const mark of [...marks].sort((a, b) => a.start - b.start || b.end - a.end)) {
    const last = out[out.length - 1]
    if (mark.start >= mark.end || (last !== undefined && mark.start < last.end)) continue
    out.push(mark)
  }
  return out
}

/** A piece of a text: under a mark, or not. */
export interface Run {
  text: string
  mark: MarkKind | null
}

/** A stretch of text cut into runs by the marks that fall on it. `offset` is
 * where the stretch starts in the text the marks count in. */
export function runsOf(text: string, offset: number, marks: readonly Mark[]): Run[] {
  const out: Run[] = []
  let at = 0
  for (const mark of marks) {
    const start = Math.max(mark.start - offset, at)
    const end = Math.min(mark.end - offset, text.length)
    if (end <= start) continue
    if (start > at) out.push({ text: text.slice(at, start), mark: null })
    out.push({ text: text.slice(start, end), mark: mark.kind })
    at = end
  }
  if (at < text.length) out.push({ text: text.slice(at), mark: null })
  return out
}

/**
 * Where each letter of a node's value stands in the passage, or `null` when
 * that cannot be told. The value is the source as written, mostly: a fenced
 * block's code is, letter for letter. A paragraph inside a quote or a list
 * item is not - its lines lose the `> ` and the indent that put them there,
 * and an escaped letter loses its backslash - so the two are walked side by
 * side, the source's letters that the value leaves out stepped over. An
 * entity, `&amp;`, is not a letter the walk can step over, and gives up.
 */
export function placesOf(value: string, source: string, start: number, end: number): number[] | null {
  if (value === '') return []
  const at = source.slice(start, end).indexOf(value)
  if (at >= 0) return Array.from({ length: value.length }, (_, index) => start + at + index)
  const places: number[] = []
  let place = start
  for (let index = 0; index < value.length; index++) {
    while (place < end && source[place] !== value[index] && /[\s>\\]/.test(source[place] ?? '')) place++
    if (place >= end || source[place] !== value[index]) return null
    places.push(place++)
  }
  return places
}

/** A stretch of text cut into runs by the marks on the places its letters
 * stand at in the passage. */
export function runsAt(text: string, places: readonly number[], marks: readonly Mark[]): Run[] {
  const out: Run[] = []
  let from = 0
  let current: MarkKind | null = null
  for (let index = 0; index < text.length; index++) {
    const place = places[index] ?? -1
    const mark = marks.find((one) => place >= one.start && place < one.end)?.kind ?? null
    if (index > 0 && mark !== current) {
      out.push({ text: text.slice(from, index), mark: current })
      from = index
    }
    current = mark
  }
  if (from < text.length) out.push({ text: text.slice(from), mark: current })
  return out
}

/** The marks that reach into a stretch, from `start` to `end`. */
export function touching(marks: readonly Mark[], start: number, end: number): Mark[] {
  return marks.filter((mark) => mark.end > start && mark.start < end)
}
