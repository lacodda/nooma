import { describe, expect, it } from 'vitest'
import { quoteOf, sourceLine } from '@/lib/quote'

describe('quoteOf', () => {
  it('quotes every line and says the file and line under it', () => {
    expect(quoteOf('Water the tomatoes.\n\nNever at noon.\n', { path: '/notes/greenhouse.md', line: 12 })).toBe(
      '> Water the tomatoes.\n>\n> Never at noon.\n>\n> `/notes/greenhouse.md:12`\n',
    )
  })

  it('keeps a Windows path as written, in code', () => {
    const windows = 'C:' + String.fromCharCode(92) + 'Notes' + String.fromCharCode(92) + '_drafts.md'
    expect(sourceLine({ path: windows, line: 3 })).toBe('`' + windows + ':3`')
  })

  it('says the page of a PDF and the headings of a book', () => {
    expect(sourceLine({ path: '/archive/handbook.pdf', page: 'p. 4' })).toBe('`/archive/handbook.pdf` · p. 4')
    expect(sourceLine({ path: '/books/cape.epub', headings: ['Winter', 'The fog signal'] })).toBe('`/books/cape.epub` · Winter › The fog signal')
    expect(sourceLine({ path: '/books/cape.epub', headings: [] })).toBe('`/books/cape.epub`')
  })

  it('fences a path that holds a backtick with more of them', () => {
    expect(sourceLine({ path: '/notes/a`b.md', line: 1 })).toBe('`` /notes/a`b.md:1 ``')
  })
})
