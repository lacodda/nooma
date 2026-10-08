import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import '@/i18n'
import { App } from '@/App'
import type { Found, MeaningStatus, Ranked, Status } from '@/lib/api'

// The webview talks to Rust through `invoke`; in jsdom there is no Rust, so
// the boundary is stubbed with the shapes `nooma-core` serializes.
const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn().mockResolvedValue(null) }))

const noSources: Status = { sources: [], documents: 0, chunks: 0, indexed_at: null, stale: null }
const oneSource: Status = { sources: [{ path: '/notes' }], documents: 3, chunks: 4, indexed_at: 1, stale: null }

const contracts: Ranked = {
  path: '/notes/договоры.md',
  kind: 'markdown',
  title: 'Договоры',
  headings: ['Договоры', 'Сроки'],
  line: 5,
  fragment: 'Договор поставки продлевается',
  highlights: [[0, 14]],
  tags: [],
  modified: 1_700_000_000,
  score: 0.26,
  words: { rank: 1, score: 2, share: 1 },
  meaning: null,
}
const other: Ranked = {
  path: '/notes/other.txt',
  kind: 'text',
  title: 'other',
  headings: [],
  line: 1,
  fragment: 'another договор',
  highlights: [],
  tags: [],
  modified: 1_700_000_000,
  score: 0.15,
  words: { rank: 2, score: 1, share: 1 },
  meaning: null,
}
const supply: Ranked = {
  path: '/notes/supply.md',
  kind: 'markdown',
  title: 'Supply terms',
  headings: [],
  line: 1,
  fragment: 'The supply agreement renews each year unless cancelled.',
  highlights: [],
  tags: [],
  modified: 1_700_000_000,
  score: 0.11,
  words: null,
  meaning: { rank: 1, score: 0.87 },
}

/** What the words alone answer. */
const byWords: Found = { hits: [contracts, other], all_words: true, by_meaning: false, behind: 0, took_ms: 3 }
/** The whole answer: the contracts found by both halves, the supply terms by
 * the meaning alone. */
const whole: Found = {
  hits: [{ ...contracts, score: 0.37, meaning: { rank: 2, score: 0.85 } }, other, supply],
  all_words: true,
  by_meaning: true,
  behind: 0,
  took_ms: 41,
}
const ready: MeaningStatus = {
  model: 'multilingual-e5-small',
  bytes: 487_000_000,
  present: true,
  fetching: false,
  passages: 3,
  behind: 0,
  computing: null,
}
const noModel: MeaningStatus = { ...ready, present: false, passages: null }

function answer(status: Status, half: MeaningStatus = ready, both: Found | null | Promise<Found | null> = whole, words: Found = byWords) {
  invoke.mockImplementation((command: string) => {
    switch (command) {
      case 'status':
        return Promise.resolve(status)
      case 'update':
        return Promise.resolve(null)
      case 'search':
        return Promise.resolve(words)
      case 'meaning_status':
        return Promise.resolve(half)
      case 'search_hybrid':
      case 'similar':
        return half.present ? Promise.resolve(both) : Promise.resolve(null)
      case 'update_vectors':
      case 'fetch_model':
        return Promise.resolve(true)
      default:
        return Promise.resolve(undefined)
    }
  })
}

beforeEach(() => invoke.mockReset())

async function typed(text: string) {
  const field = await screen.findByRole('searchbox', { name: 'Search your files' })
  fireEvent.change(field, { target: { value: text } })
  return field
}

async function press(field: HTMLElement, key: string) {
  await act(async () => {
    fireEvent.keyDown(field, { key })
  })
}

describe('App', () => {
  it('asks for a folder when there is nothing to search', async () => {
    answer(noSources)
    render(<App />)
    expect(await screen.findByRole('button', { name: 'Add folder' })).toBeTruthy()
    expect(invoke).not.toHaveBeenCalledWith('update')
  })

  it('catches the index up on opening, and then the vectors', async () => {
    answer(oneSource)
    render(<App />)
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('update'))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('update_vectors'))
  })

  it('does not ask for vectors when there is no model to compute them', async () => {
    answer(oneSource, noModel)
    render(<App />)
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('meaning_status'))
    expect(invoke).not.toHaveBeenCalledWith('update_vectors')
  })

  it('shows results with the matched word marked, in Russian', async () => {
    answer(oneSource)
    render(<App />)
    await typed('договоров')
    const mark = await screen.findByText('Договор', { selector: 'mark' })
    expect(mark).toBeTruthy()
    expect(invoke).toHaveBeenCalledWith('search', { query: 'договоров' })
    expect(invoke).toHaveBeenCalledWith('search_hybrid', { query: 'договоров' })
  })

  it('shows one list from both halves, each document once, marked by the halves that found it', async () => {
    answer(oneSource)
    render(<App />)
    await typed('договор')
    await screen.findByText('Supply terms')
    const list = screen.getByRole('list', { name: 'Results' })
    const rows = list.querySelectorAll('li')
    expect(rows).toHaveLength(3)
    expect(screen.getByText('3 found')).toBeTruthy()
    // The contracts were found by both halves, the supply terms by meaning.
    expect(within(rows[0] as HTMLElement).getByText('words')).toBeTruthy()
    expect(within(rows[0] as HTMLElement).getByText('meaning')).toBeTruthy()
    expect(within(rows[2] as HTMLElement).queryByText('words')).toBeNull()
    // The cosine is in the meaning's hint, not on the row.
    expect(within(rows[2] as HTMLElement).getByText('meaning').getAttribute('title')).toContain('0.87')
    expect(list.textContent).not.toContain('0.87')
  })

  it('shows the words at once while the whole answer is slow, and keeps the selection when it comes', async () => {
    let arrive: (found: Found) => void = () => undefined
    const slow = new Promise<Found>((resolve) => {
      arrive = resolve
    })
    answer(oneSource, ready, slow)
    render(<App />)
    const field = await typed('договор')
    // The words stand in once the grace period has passed.
    await screen.findByText('other')
    expect(screen.queryByText('Supply terms')).toBeNull()
    await press(field, 'ArrowDown')
    await act(async () => {
      arrive({ ...whole, hits: [supply, { ...contracts, meaning: { rank: 2, score: 0.85 } }, other] })
    })
    await screen.findByText('Supply terms')
    fireEvent.keyDown(field, { key: 'Enter' })
    expect(invoke).toHaveBeenCalledWith('open_document', { path: '/notes/other.txt' })
  })

  it('opens the selected result on Enter and reveals it on Ctrl+Enter', async () => {
    answer(oneSource)
    render(<App />)
    const field = await typed('договор')
    await screen.findByText('Supply terms')
    await press(field, 'ArrowDown')
    await press(field, 'ArrowDown')
    fireEvent.keyDown(field, { key: 'Enter' })
    expect(invoke).toHaveBeenCalledWith('open_document', { path: '/notes/supply.md' })
    fireEvent.keyDown(field, { key: 'Enter', ctrlKey: true })
    expect(invoke).toHaveBeenCalledWith('reveal_document', { path: '/notes/supply.md' })
  })

  it('says when no document holds every word, and marks the rows that hold some', async () => {
    const some: Found = { ...whole, all_words: false, hits: [{ ...supply }, { ...other, words: { rank: 1, score: 1, share: 0.5 } }] }
    answer(oneSource, ready, some)
    render(<App />)
    await typed('договор zeppelin')
    expect(await screen.findByText(/no document has every word - these have some/)).toBeTruthy()
    expect(screen.getByText('some words')).toBeTruthy()
  })

  it('answers with the words when there is no model, and offers it', async () => {
    answer(oneSource, noModel)
    render(<App />)
    await typed('договор')
    expect(await screen.findByText('Search by meaning needs its model')).toBeTruthy()
    expect(screen.getByRole('list', { name: 'Results' }).querySelectorAll('li')).toHaveLength(2)
    fireEvent.click(screen.getByRole('button', { name: 'Download the model' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('fetch_model'))
  })

  it('takes a pasted paragraph as a passage to find more like, not as a query', async () => {
    answer(oneSource, ready, { ...whole, hits: [supply] })
    render(<App />)
    const field = await screen.findByRole('searchbox', { name: 'Search your files' })
    const passage = 'The supply agreement renews each year.\nIt can be cancelled in writing.'
    fireEvent.paste(field, { clipboardData: { getData: () => passage } })
    expect(await screen.findByText('Similar to the pasted passage')).toBeTruthy()
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('similar', { text: passage }))
    expect(await screen.findByText('Supply terms')).toBeTruthy()
    expect(invoke).not.toHaveBeenCalledWith('search', expect.anything())
    expect((field as HTMLInputElement).value).toBe('The supply agreement renews each year. It can be cancelled in writing.')
  })

  it('takes a short paste as a query', async () => {
    answer(oneSource)
    render(<App />)
    const field = await screen.findByRole('searchbox', { name: 'Search your files' })
    fireEvent.paste(field, { clipboardData: { getData: () => 'INV-2025-0114' } })
    expect(invoke).not.toHaveBeenCalledWith('similar', expect.anything())
  })
})
