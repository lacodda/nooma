import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import '@/i18n'
import { App } from '@/App'
import type { Found, MeaningFound, MeaningStatus, Status } from '@/lib/api'

// The webview talks to Rust through `invoke`; in jsdom there is no Rust, so
// the boundary is stubbed with the shapes `nooma-core` serializes.
const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn().mockResolvedValue(null) }))

const noSources: Status = { sources: [], documents: 0, chunks: 0, indexed_at: null, stale: null }
const oneSource: Status = { sources: [{ path: '/notes' }], documents: 2, chunks: 3, indexed_at: 1, stale: null }
const found: Found = {
  took_ms: 3,
  hits: [
    {
      path: '/notes/договоры.md',
      kind: 'markdown',
      title: 'Договоры',
      headings: ['Договоры', 'Сроки'],
      line: 5,
      fragment: 'Договор поставки продлевается',
      highlights: [[0, 14]],
      tags: [],
      score: 2,
    },
    {
      path: '/notes/other.txt',
      kind: 'text',
      title: 'other',
      headings: [],
      line: 1,
      fragment: 'another договор',
      highlights: [],
      tags: [],
      score: 1,
    },
  ],
}
const meaning: MeaningFound = {
  took_ms: 41,
  behind: 0,
  hits: [
    {
      path: '/notes/supply.md',
      kind: 'markdown',
      title: 'Supply terms',
      headings: [],
      line: 1,
      fragment: 'The supply agreement renews each year unless cancelled.',
      highlights: [],
      tags: [],
      score: 0.87,
    },
  ],
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

function answer(status: Status, half: MeaningStatus = ready, byMeaning: MeaningFound | null = meaning, exact: Found = found) {
  invoke.mockImplementation((command: string) => {
    switch (command) {
      case 'status':
        return Promise.resolve(status)
      case 'update':
        return Promise.resolve(null)
      case 'search':
        return Promise.resolve(exact)
      case 'meaning_status':
        return Promise.resolve(half)
      case 'search_meaning':
      case 'similar':
        return Promise.resolve(half.present ? byMeaning : null)
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
  })

  it('shows the exact list and the meaning list, each under its heading', async () => {
    answer(oneSource)
    render(<App />)
    await typed('договор')
    const exact = await screen.findByRole('list', { name: 'Exact matches' })
    const byMeaning = await screen.findByRole('list', { name: 'By meaning' })
    expect(exact.querySelectorAll('li')).toHaveLength(2)
    expect(byMeaning.textContent).toContain('Supply terms')
    // A cosine means something on its own, so the meaning list shows it.
    expect(byMeaning.textContent).toContain('0.87')
    expect(exact.textContent).not.toContain('2.00')
    expect(invoke).toHaveBeenCalledWith('search_meaning', { query: 'договор' })
  })

  it('opens the selected result on Enter and reveals it on Ctrl+Enter', async () => {
    answer(oneSource)
    render(<App />)
    const field = await typed('договор')
    await screen.findByText('other')
    await act(async () => {
      fireEvent.keyDown(field, { key: 'ArrowDown' })
    })
    fireEvent.keyDown(field, { key: 'Enter' })
    expect(invoke).toHaveBeenCalledWith('open_document', { path: '/notes/other.txt' })
    fireEvent.keyDown(field, { key: 'Enter', ctrlKey: true })
    expect(invoke).toHaveBeenCalledWith('reveal_document', { path: '/notes/other.txt' })
  })

  it('moves from the last exact match to the first by meaning', async () => {
    answer(oneSource)
    render(<App />)
    const field = await typed('договор')
    await screen.findByText('Supply terms')
    for (let i = 0; i < 2; i++) {
      await act(async () => {
        fireEvent.keyDown(field, { key: 'ArrowDown' })
      })
    }
    fireEvent.keyDown(field, { key: 'Enter' })
    expect(invoke).toHaveBeenCalledWith('open_document', { path: '/notes/supply.md' })
  })

  it('shows five rows of each list until asked for the rest, so both lists fit one screen', async () => {
    const many: Found = {
      took_ms: 2,
      hits: Array.from({ length: 7 }, (_, i) => ({
        path: `/notes/n${i}.txt`,
        kind: 'text' as const,
        title: `note ${i}`,
        headings: [],
        line: 1,
        fragment: 'a note',
        highlights: [],
        tags: [],
        score: 7 - i,
      })),
    }
    answer(oneSource, ready, meaning, many)
    render(<App />)
    await typed('note')
    const exact = await screen.findByRole('list', { name: 'Exact matches' })
    await screen.findByText('Supply terms')
    expect(exact.querySelectorAll('li')).toHaveLength(5)
    fireEvent.click(screen.getByRole('button', { name: 'Show all 7' }))
    await waitFor(() => expect(exact.querySelectorAll('li')).toHaveLength(7))
  })

  it('offers the model when there is none, and fetches it when asked', async () => {
    answer(oneSource, noModel)
    render(<App />)
    await typed('договор')
    expect(await screen.findByText('Search by meaning needs its model')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Download the model' }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('fetch_model'))
  })

  it('takes a pasted paragraph as a passage to find more like, not as a query', async () => {
    answer(oneSource)
    render(<App />)
    const field = await screen.findByRole('searchbox', { name: 'Search your files' })
    const passage = 'The supply agreement renews each year.\nIt can be cancelled in writing.'
    fireEvent.paste(field, { clipboardData: { getData: () => passage } })
    expect(await screen.findByText('Similar to the pasted passage')).toBeTruthy()
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('similar', { text: passage }))
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
