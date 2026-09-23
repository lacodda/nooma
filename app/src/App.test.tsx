import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import '@/i18n'
import { App } from '@/App'
import type { Found, Status } from '@/lib/api'

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

function answer(status: Status) {
  invoke.mockImplementation((command: string) => {
    switch (command) {
      case 'status':
        return Promise.resolve(status)
      case 'update':
        return Promise.resolve(null)
      case 'search':
        return Promise.resolve(found)
      default:
        return Promise.resolve(undefined)
    }
  })
}

beforeEach(() => invoke.mockReset())

describe('App', () => {
  it('asks for a folder when there is nothing to search', async () => {
    answer(noSources)
    render(<App />)
    expect(await screen.findByRole('button', { name: 'Add folder' })).toBeTruthy()
    expect(invoke).not.toHaveBeenCalledWith('update')
  })

  it('catches the index up on opening when there are folders', async () => {
    answer(oneSource)
    render(<App />)
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('update'))
  })

  it('shows results with the matched word marked, in Russian', async () => {
    answer(oneSource)
    render(<App />)
    const field = await screen.findByRole('searchbox', { name: 'Search your files' })
    fireEvent.change(field, { target: { value: 'договоров' } })
    const mark = await screen.findByText('Договор', { selector: 'mark' })
    expect(mark).toBeTruthy()
    expect(invoke).toHaveBeenCalledWith('search', { query: 'договоров' })
  })

  it('opens the selected result on Enter and reveals it on Ctrl+Enter', async () => {
    answer(oneSource)
    render(<App />)
    const field = await screen.findByRole('searchbox', { name: 'Search your files' })
    fireEvent.change(field, { target: { value: 'договор' } })
    await screen.findByText('other')
    await act(async () => {
      fireEvent.keyDown(field, { key: 'ArrowDown' })
    })
    fireEvent.keyDown(field, { key: 'Enter' })
    expect(invoke).toHaveBeenCalledWith('open_document', { path: '/notes/other.txt' })
    fireEvent.keyDown(field, { key: 'Enter', ctrlKey: true })
    expect(invoke).toHaveBeenCalledWith('reveal_document', { path: '/notes/other.txt' })
  })
})
