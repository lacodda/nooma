import { describe, expect, it } from 'vitest'
import { PASSAGE_CHARS, flatten, isPassage } from '@/lib/passage'

describe('isPassage', () => {
  it('takes a paste of more than one line as a passage', () => {
    expect(isPassage('First line.\nSecond line.')).toBe(true)
    expect(isPassage('First line.\r\nSecond line.')).toBe(true)
  })

  it('takes a long paste as a passage, and a short one as a query', () => {
    expect(isPassage('x'.repeat(PASSAGE_CHARS))).toBe(true)
    expect(isPassage('x'.repeat(PASSAGE_CHARS - 1))).toBe(false)
    expect(isPassage('INV-2025-0114')).toBe(false)
  })

  it('does not count a line break the clipboard left at the end', () => {
    expect(isPassage('договор поставки\n')).toBe(false)
  })
})

describe('flatten', () => {
  it('puts a passage on one line', () => {
    expect(flatten('  Water the tomatoes\n\nin the morning.  ')).toBe('Water the tomatoes in the morning.')
  })
})
