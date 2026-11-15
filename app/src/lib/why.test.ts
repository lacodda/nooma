import { describe, expect, it, vi } from 'vitest'
import type { Explained, Ranked } from '@/lib/api'
import { askOnce, wantsWhy } from '@/lib/why'

function row(changes: Partial<Ranked>): Ranked {
  return {
    path: '/notes/a.md',
    kind: 'markdown',
    title: 'a',
    headings: [],
    line: 1,
    fragment: 'Descale the kettle.',
    highlights: [],
    tags: [],
    modified: 0,
    score: 1,
    words: null,
    meaning: { rank: 1, score: 0.8 },
    across: false,
    ...changes,
  }
}

describe('wantsWhy', () => {
  it('asks of a row the meaning found in the question’s own language too', () => {
    expect(wantsWhy(row({}))).toBe(true)
    expect(wantsWhy(row({ across: true }))).toBe(true)
  })

  it('does not ask of a row that is marked, told, or the words’ alone', () => {
    expect(wantsWhy(row({ highlights: [[0, 7]] }))).toBe(false)
    expect(wantsWhy(row({ closest: [0, 7] }))).toBe(false)
    expect(wantsWhy(row({ meaning: null, words: { rank: 1, score: 3, share: 1 } }))).toBe(false)
  })
})

describe('askOnce', () => {
  it('asks the model once for a row, however many ask', async () => {
    const told: Explained = { fragment: 'Descale the kettle.', closest: [0, 19] }
    const ask = vi.fn(() => Promise.resolve(told))
    const why = askOnce(ask)
    const [first, second] = await Promise.all([why(row({})), why(row({}))])
    expect(first).toBe(told)
    expect(second).toBe(told)
    expect(ask).toHaveBeenCalledTimes(1)
    await why(row({ line: 9 }))
    expect(ask).toHaveBeenCalledTimes(2)
  })

  it('gives every asker the same failure', async () => {
    const ask = vi.fn(() => Promise.reject(new Error('the model failed')))
    const why = askOnce(ask)
    await expect(why(row({}))).rejects.toThrow('the model failed')
    await expect(why(row({}))).rejects.toThrow('the model failed')
    expect(ask).toHaveBeenCalledTimes(1)
  })
})
