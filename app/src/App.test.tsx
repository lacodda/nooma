import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import '@/i18n'
import { App } from '@/App'
import type { Found, Health, IndexState, MeaningStatus, Ranked, Shown, Status } from '@/lib/api'

// The webview talks to Rust through `invoke`; in jsdom there is no Rust, so
// the boundary is stubbed with the shapes `nooma-core` serializes. The events
// Rust sends are kept by name, for a test to send one as the window would.
const invoke = vi.fn()
const listeners = new Map<string, (event: { payload: unknown }) => void>()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((name: string, handler: (event: { payload: unknown }) => void) => {
    listeners.set(name, handler)
    return Promise.resolve(() => undefined)
  }),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn().mockResolvedValue(null) }))
// Rust is there, as far as the page asks.
Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    show: () => Promise.resolve(),
    isMaximized: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => undefined),
  }),
}))

/** Send an event as the window's Rust side sends it. */
async function sent(name: string, payload: unknown) {
  await waitFor(() => expect(listeners.has(name)).toBe(true))
  await act(async () => listeners.get(name)?.({ payload }))
}

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
  across: false,
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
  across: false,
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
  across: false,
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

/** The contracts note around the chunk found: the chunk before it and the
 * one after, the query's word marked in bytes of UTF-8. */
const contractsShown: Shown = {
  path: contracts.path,
  kind: 'markdown',
  title: 'Договоры',
  passages: [
    { headings: ['Договоры', 'Стороны'], line: 3, text: 'Поставщик и покупатель.', highlights: [] },
    // `Договор` is fourteen bytes: seven Cyrillic letters of two each.
    { headings: ['Договоры', 'Сроки'], line: 5, text: 'Договор поставки продлевается.\n\n- [x] подписан\n- [ ] [[Счета#Март|счёт]] #финансы', highlights: [[0, 14]] },
    { headings: ['Договоры', 'Сроки'], line: 11, text: 'Срок - год.', highlights: [] },
  ],
  found: 1,
  before: 0,
  after: 3,
  opens: { program: 'Visual Studio Code', at: { kind: 'line', line: 5 } },
}

const idle: IndexState = { kind: 'idle', passes: 0, watching: false, seq: 0 }
const healthy: Health = { skipped: [], textless: [], unrecognized: [], guessed: [], unavailable: [] }
let health: Health = healthy

/** What the preview is told about a document. */
let shownFor: (args: { path: string }) => Shown | null = () => null

function answer(status: Status, half: MeaningStatus = ready, both: Found | null | Promise<Found | null> = whole, words: Found = byWords) {
  invoke.mockImplementation((command: string, args?: unknown) => {
    switch (command) {
      case 'status':
        return Promise.resolve(status)
      case 'index_state':
        return Promise.resolve(idle)
      case 'health':
        return Promise.resolve(health)
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
      case 'preview':
        return Promise.resolve(shownFor(args as { path: string }))
      default:
        return Promise.resolve(undefined)
    }
  })
}

/** A window as wide as it opens, or one at its narrowest. */
function windowWide(wide: boolean) {
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    value: (query: string) => ({ matches: wide, media: query, addEventListener: () => undefined, removeEventListener: () => undefined }),
  })
}

beforeEach(() => {
  invoke.mockReset()
  listeners.clear()
  health = healthy
  shownFor = (args) => (args.path === contracts.path ? contractsShown : null)
  windowWide(false)
})

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
  })

  it('follows the indexing the window runs, and reads what it changed when it is done', async () => {
    answer(oneSource)
    render(<App />)
    await screen.findByRole('img', { name: 'index up to date' })
    const statusAsked = () => invoke.mock.calls.filter(([command]) => command === 'status').length
    const before = statusAsked()

    await sent('index-state', { kind: 'running', done: 2, total: 5, passes: 0, watching: false, seq: 3 })
    expect(screen.getByRole('img', { name: 'indexing 2 / 5' })).toBeTruthy()
    // A state older than the one shown is not drawn over it.
    await sent('index-state', { ...idle, seq: 1 })
    expect(screen.getByRole('img', { name: 'indexing 2 / 5' })).toBeTruthy()

    await sent('index-state', { kind: 'idle', passes: 1, watching: true, seq: 4 })
    expect(screen.getByRole('img', { name: 'index up to date' }).parentElement?.title).toBe(
      'Watching the folders: a file saved is found a moment later.',
    )
    await waitFor(() => expect(statusAsked()).toBeGreaterThan(before))
    expect(invoke).toHaveBeenCalledWith('health')
  })

  it('says when indexing waits for the power', async () => {
    answer(oneSource)
    render(<App />)
    await sent('index-state', { kind: 'held', passes: 0, watching: true, seq: 2 })
    expect(screen.getByRole('img', { name: 'indexing paused: on battery' })).toBeTruthy()
    await sent('vectors-held', null)
    expect(screen.getByRole('img', { name: 'meaning paused: on battery' })).toBeTruthy()
  })

  it('opens what the index lacks from the status line, grouped by why', async () => {
    health = {
      ...healthy,
      skipped: [
        { path: '/notes/tax-return.pdf', why: 'locked', reason: 'protected by a password', size: 4096 },
        { path: '/notes/video/talk.txt', why: 'too_large', reason: 'larger than 16 MB', size: 40 * 1024 * 1024 },
      ],
      guessed: [{ path: '/notes/сторож.txt', encoding: 'windows-1251' }],
      unavailable: ['/archive'],
    }
    answer(oneSource)
    render(<App />)
    const chip = await screen.findByRole('button', { name: '4 to check' })
    fireEvent.click(chip)
    const panel = await screen.findByRole('dialog', { name: 'What the index lacks' })
    const locked = within(panel).getByRole('region', { name: 'Under a password or DRM · 1' })
    const row = within(locked).getByRole('button', { name: 'Show tax-return.pdf in its folder' })
    expect(row.closest('li')?.title).toBe('/notes/tax-return.pdf\nprotected by a password')
    const large = within(panel).getByRole('region', { name: 'Too large · 1' })
    expect(within(large).getByText('40 MB')).toBeTruthy()
    expect(within(large).getByText('notes/video')).toBeTruthy()
    expect(within(panel).getByText('windows-1251')).toBeTruthy()
    // A folder out of reach is said, not offered to be shown.
    const away = within(panel).getByRole('region', { name: 'Folders out of reach · 1' })
    expect(within(away).queryByRole('button')).toBeNull()

    fireEvent.click(row)
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('reveal_document', { path: '/notes/tax-return.pdf' }))
  })

  it('offers no panel when the index lacks nothing', async () => {
    answer(oneSource)
    render(<App />)
    await screen.findByRole('img', { name: 'index up to date' })
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('health'))
    expect(screen.queryByRole('button', { name: /to check/ })).toBeNull()
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

  it('says where a PDF answered by its page, and where a note did by its line', async () => {
    const manual: Ranked = {
      ...supply,
      path: '/notes/manuals/kettle.pdf',
      kind: 'pdf',
      title: 'kettle',
      headings: ['Care', 'Descaling'],
      line: 340,
      page: 12,
    }
    answer(oneSource, ready, { ...whole, hits: [manual, contracts] })
    render(<App />)
    await typed('descaling')
    const row = (await screen.findByText('kettle')).closest('li') as HTMLElement
    expect(within(row).getByText('PDF')).toBeTruthy()
    expect(row.textContent).toContain('Care › Descaling · p. 12')
    expect(row.textContent).not.toContain('line 340')
    const note = screen.getByText('Договоры').closest('li') as HTMLElement
    expect(within(note).getByText('MD')).toBeTruthy()
    expect(note.textContent).toContain('line 5')
  })

  it('says where a Word document or a book answered by its headings alone', async () => {
    const lease: Ranked = { ...supply, path: '/notes/lease', kind: 'docx', title: 'lease', headings: ['Terms', 'Rent'], line: 9 }
    const book: Ranked = { ...supply, path: '/notes/book.epub', kind: 'epub', title: 'Notebook', headings: ['Winter'], line: 21 }
    answer(oneSource, ready, { ...whole, hits: [lease, book] })
    render(<App />)
    await typed('rent')
    const row = (await screen.findByText('lease')).closest('li') as HTMLElement
    expect(within(row).getByText('DOCX')).toBeTruthy()
    expect(row.textContent).toContain('Terms › Rent')
    expect(row.textContent).not.toContain('line 9')
    const chapter = screen.getByText('Notebook').closest('li') as HTMLElement
    expect(within(chapter).getByText('EPUB')).toBeTruthy()
    expect(chapter.textContent).not.toContain('line 21')
  })

  it('says a row read by OCR was, for a picture and for a scanned page', async () => {
    const shot: Ranked = { ...supply, path: '/notes/shots/backup.png', kind: 'image', title: 'backup', headings: [], line: 3, recognized: true }
    const scan: Ranked = { ...supply, path: '/notes/lease.pdf', kind: 'pdf', title: 'lease', headings: [], line: 40, page: 2, recognized: true }
    answer(oneSource, ready, { ...whole, hits: [shot, scan, contracts] })
    render(<App />)
    await typed('backup drive')
    const picture = (await screen.findByText('backup')).closest('li') as HTMLElement
    expect(within(picture).getByText('IMG')).toBeTruthy()
    expect(within(picture).getByText('OCR').getAttribute('title')).toContain('misread')
    expect(picture.textContent).not.toContain('line 3')
    const page = screen.getByText('lease').closest('li') as HTMLElement
    expect(within(page).getByText('OCR')).toBeTruthy()
    expect(page.textContent).toContain('p. 2')
    const note = screen.getByText('Договоры').closest('li') as HTMLElement
    expect(within(note).queryByText('OCR')).toBeNull()
  })

  it('tells a row found in another language why: the sentence that answered, marked', async () => {
    const english: Ranked = { ...supply, across: true, fragment: 'Terms. The supply agreement renews each year unless cancelled.' }
    answer(oneSource, ready, { ...whole, hits: [contracts, english] })
    const sentence = 'The supply agreement renews each year unless cancelled.'
    // Byte offsets: the ellipsis before the sentence is three bytes of UTF-8.
    const told = { fragment: `…${sentence}`, closest: [3, 3 + sentence.length] }
    const plain = invoke.getMockImplementation()
    invoke.mockImplementation((command: string, args?: unknown) =>
      command === 'explain' ? Promise.resolve(told) : (plain as (c: string, a?: unknown) => unknown)(command, args),
    )
    render(<App />)
    await typed('договор поставки')
    const mark = await screen.findByText(sentence, { selector: 'mark' })
    expect(mark.className).toContain('bg-accent-soft')
    expect(invoke).toHaveBeenCalledWith('explain', { text: 'договор поставки', passage: false, path: '/notes/supply.md', line: 1 })
    expect(invoke.mock.calls.filter(([command]) => command === 'explain')).toHaveLength(1)
    const row = screen.getByText('Supply terms').closest('li') as HTMLElement
    expect(within(row).getByText('another language').getAttribute('title')).toContain('another language than the query')
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
    expect(invoke).toHaveBeenCalledWith('open_document', { path: '/notes/other.txt', line: 1 })
  })

  it('opens the selected result on Enter and reveals it on Ctrl+Enter', async () => {
    answer(oneSource)
    render(<App />)
    const field = await typed('договор')
    await screen.findByText('Supply terms')
    await press(field, 'ArrowDown')
    await press(field, 'ArrowDown')
    fireEvent.keyDown(field, { key: 'Enter' })
    expect(invoke).toHaveBeenCalledWith('open_document', { path: '/notes/supply.md', line: 1 })
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

  it('reads the row selected in its document beside the list, the note drawn as a note', async () => {
    windowWide(true)
    answer(oneSource)
    render(<App />)
    await typed('договор')
    const preview = await screen.findByRole('region', { name: 'Preview' })
    await within(preview).findByText('Поставщик и покупатель.')
    // The section the found chunk starts, under the note's title.
    expect(within(preview).getByRole('heading', { name: 'Сроки' })).toBeTruthy()
    expect(within(preview).getAllByRole('heading', { name: 'Договоры' })).toHaveLength(1)
    expect(within(preview).getByRole('heading', { name: 'Стороны' })).toBeTruthy()
    // The word found is marked in the whole chunk, the chunk is marked as
    // the one found, and the note's markdown reads as a note.
    const found = preview.querySelector('[data-found]') as HTMLElement
    expect(within(found).getByText('Договор', { selector: 'mark' })).toBeTruthy()
    expect(within(found).getAllByRole('checkbox').map((box) => (box as HTMLInputElement).checked)).toEqual([true, false])
    expect(within(found).getByText('счёт').getAttribute('title')).toBe('Счета#Март')
    expect(within(found).getByText('#финансы')).toBeTruthy()
    expect(found.textContent).not.toContain('[[')

    const open = within(preview).getByRole('button', { name: /Open at line 5/ })
    expect(open.getAttribute('title')).toBe('In Visual Studio Code')
    fireEvent.click(open)
    expect(invoke).toHaveBeenCalledWith('open_document', { path: contracts.path, line: 5 })
    expect(invoke).toHaveBeenCalledWith('preview', { path: contracts.path, line: 5, query: 'договор', before: 2, after: 2 })

    fireEvent.click(within(preview).getByRole('button', { name: /Show 3 later passages/ }))
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('preview', { path: contracts.path, line: 5, query: 'договор', before: 2, after: 6 }))
  })

  it('marks the sentence that answered in the preview of a row found by meaning alone', async () => {
    windowWide(true)
    const passage = 'Terms. The supply agreement renews each year unless cancelled. Notice in writing.'
    const sentence = 'The supply agreement renews each year unless cancelled.'
    shownFor = (args) =>
      args.path === supply.path
        ? {
            path: supply.path,
            kind: 'text',
            title: 'Supply terms',
            passages: [{ headings: [], line: 1, text: passage, highlights: [] }],
            found: 0,
            before: 0,
            after: 0,
            opens: { program: null, at: null },
          }
        : null
    answer(oneSource, ready, { ...whole, hits: [supply, contracts] })
    const plain = invoke.getMockImplementation()
    invoke.mockImplementation((command: string, args?: unknown) =>
      command === 'explain'
        ? Promise.resolve({ fragment: `…${sentence}`, closest: [3, 3 + sentence.length] })
        : (plain as (c: string, a?: unknown) => unknown)(command, args),
    )
    render(<App />)
    await typed('договор поставки')
    const preview = await screen.findByRole('region', { name: 'Preview' })
    const mark = await within(preview).findByText(sentence, { selector: 'mark' })
    expect(mark.className).toContain('bg-accent-soft')
    expect(invoke).toHaveBeenCalledWith('explain', { text: 'договор поставки', passage: false, path: supply.path, line: 1 })
    // A program the system does not name: the button says only what it does.
    expect(within(preview).getByRole('button', { name: /^Open/ }).getAttribute('title')).toBe('In the program this kind of file opens with')
  })

  it('leaves the list the window at its narrowest', async () => {
    answer(oneSource)
    render(<App />)
    await typed('договор')
    await screen.findByText('Supply terms')
    expect(screen.queryByRole('region', { name: 'Preview' })).toBeNull()
    expect(invoke).not.toHaveBeenCalledWith('preview', expect.anything())
  })

  it('takes the row selected away as a quote, with the file and its line under it', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })
    answer(oneSource)
    render(<App />)
    const field = await typed('договор')
    await screen.findByText('Supply terms')
    // The window is narrow: the chunk is asked for, alone, to be quoted.
    await act(async () => {
      fireEvent.keyDown(field, { key: 'C', code: 'KeyC', ctrlKey: true, shiftKey: true })
    })
    await waitFor(() => expect(writeText).toHaveBeenCalled())
    expect(invoke).toHaveBeenCalledWith('preview', { path: contracts.path, line: 5, query: null, before: 0, after: 0 })
    expect(writeText).toHaveBeenCalledWith(
      ['> Договор поставки продлевается.', '>', '> - [x] подписан', '> - [ ] [[Счета#Март|счёт]] #финансы', '>', '> `/notes/договоры.md:5`', ''].join('\n'),
    )
    expect(await screen.findByText('Copied', { selector: '[role="status"]' })).toBeTruthy()
  })

  it('takes a short paste as a query', async () => {
    answer(oneSource)
    render(<App />)
    const field = await screen.findByRole('searchbox', { name: 'Search your files' })
    fireEvent.paste(field, { clipboardData: { getData: () => 'INV-2025-0114' } })
    expect(invoke).not.toHaveBeenCalledWith('similar', expect.anything())
  })
})
