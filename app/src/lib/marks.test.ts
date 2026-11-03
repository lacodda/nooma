import { describe, expect, it } from 'vitest'
import { ordered, runsOf, sentenceIn, utf16Ranges } from '@/lib/marks'

describe('utf16Ranges', () => {
  it('turns bytes of UTF-8 into the string indexes of the same letters', () => {
    // `Договор` is fourteen bytes and seven letters; the emoji is four bytes
    // and two UTF-16 units.
    const text = 'Договор 🌱 contract'
    const [word, plant, english] = utf16Ranges(text, [
      [0, 14],
      [15, 19],
      [20, 28],
    ])
    expect(text.slice(...word!)).toBe('Договор')
    expect(text.slice(...plant!)).toBe('🌱')
    expect(text.slice(...english!)).toBe('contract')
  })

  it('drops a range that does not fall on the edges of letters', () => {
    expect(utf16Ranges('Договор', [[1, 3]])).toEqual([])
  })
})

describe('sentenceIn', () => {
  it('finds the sentence a fragment marks in the whole passage', () => {
    const passage = 'Terms. The supply agreement renews each year. Ends in May.'
    const sentence = 'The supply agreement renews each year.'
    // The fragment starts with an ellipsis: three bytes of UTF-8.
    expect(sentenceIn(passage, `…${sentence} Ends…`, [3, 3 + sentence.length])).toEqual([7, 7 + sentence.length])
    expect(sentenceIn('another text', `…${sentence}`, [3, 3 + sentence.length])).toBeNull()
  })
})

describe('runsOf', () => {
  it('cuts a stretch of text by the marks on it, counted from where it starts', () => {
    const marks = ordered([
      { start: 14, end: 18, kind: 'word' },
      { start: 2, end: 6, kind: 'word' },
      { start: 3, end: 5, kind: 'closest' },
    ])
    expect(marks).toHaveLength(2)
    expect(runsOf('open the vents', 10, marks)).toEqual([
      { text: 'open', mark: null },
      { text: ' the', mark: 'word' },
      { text: ' vents', mark: null },
    ])
  })
})
