import { describe, expect, it } from 'vitest'
import { splitHighlights } from '@/lib/highlight'

describe('splitHighlights', () => {
  it('marks by byte offsets in English', () => {
    expect(splitHighlights('the warranty claim', [[4, 12]])).toEqual([
      { text: 'the ', marked: false },
      { text: 'warranty', marked: true },
      { text: ' claim', marked: false },
    ])
  })

  // Every Cyrillic letter is two bytes: treating the offsets as string
  // indexes would mark "поставки продле" instead of the word.
  it('marks by byte offsets in Russian', () => {
    const fragment = 'Договор поставки'
    const start = new TextEncoder().encode('Договор ').length
    const end = new TextEncoder().encode(fragment).length
    expect(splitHighlights(fragment, [[0, 14]]).find((p) => p.marked)?.text).toBe('Договор')
    expect(splitHighlights(fragment, [[start, end]]).find((p) => p.marked)?.text).toBe('поставки')
  })

  it('ignores ranges that overlap or run past the end', () => {
    const pieces = splitHighlights('abc def', [
      [0, 3],
      [1, 2],
      [4, 99],
    ])
    expect(pieces).toEqual([
      { text: 'abc', marked: true },
      { text: ' def', marked: false },
    ])
  })
})
