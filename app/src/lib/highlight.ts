/*
 * The core reports highlights as byte offsets into UTF-8, which is what Rust
 * strings are indexed by; a JavaScript string is indexed by UTF-16 units. In
 * English the two agree, which is exactly why getting this wrong survives
 * until the first Russian result, where every letter is two bytes and every
 * mark lands in the wrong place.
 */

export interface Piece {
  text: string
  marked: boolean
}

/** Cut a fragment into plain and marked pieces, in order. */
export function splitHighlights(fragment: string, ranges: readonly (readonly [number, number])[]): Piece[] {
  const bytes = new TextEncoder().encode(fragment)
  const decoder = new TextDecoder()
  const sorted = [...ranges]
    .filter(([start, end]) => start < end && end <= bytes.length)
    .sort((a, b) => a[0] - b[0])

  const pieces: Piece[] = []
  let at = 0
  for (const [start, end] of sorted) {
    if (start < at) continue
    if (start > at) pieces.push({ text: decoder.decode(bytes.subarray(at, start)), marked: false })
    pieces.push({ text: decoder.decode(bytes.subarray(start, end)), marked: true })
    at = end
  }
  if (at < bytes.length) pieces.push({ text: decoder.decode(bytes.subarray(at)), marked: false })
  return pieces
}
