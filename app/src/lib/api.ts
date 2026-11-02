import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

/*
 * The window's side of the commands in `src-tauri/src/commands.rs`. The shapes
 * mirror `nooma-core`'s serialized types, the same ones `nooma find --json`
 * prints, so the window and the CLI describe a hit in one vocabulary.
 */

export interface Source {
  path: string
  exclude?: string[]
}

export interface Status {
  sources: Source[]
  documents: number
  chunks: number
  indexed_at: number | null
  stale: string | null
}

export interface Hit {
  path: string
  /** What the file is, as its bytes told it. */
  kind: 'markdown' | 'text' | 'pdf' | 'docx' | 'epub' | 'image'
  title: string
  headings: string[]
  /** Where the chunk is: a line of a note, and for the other kinds a place
   * the library keys the chunk by rather than one shown. */
  line: number
  /** The page the chunk is on, from 1; only a PDF's hit has one. */
  page?: number
  /** Whether the chunk's text was read by OCR out of a picture - a picture
   * file, or a PDF page that is one - and so may hold misread words. */
  recognized?: boolean
  fragment: string
  /** Byte offsets into the UTF-8 fragment, `[start, end]`. */
  highlights: [number, number][]
  /** Where the sentence closest in meaning to the query is in the fragment,
   * in the same offsets; once the row has been asked why it was found. */
  closest?: [number, number]
  tags: string[]
  /** When the file was last modified, in seconds since the epoch. */
  modified: number
  score: number
}

/** Where a document stood in one half's list. */
export interface Place {
  /** From 1. */
  rank: number
  /** The half's own score: full-text, or a cosine. */
  score: number
  /** For the words: how much of the query the chunk holds, from 0 to 1. */
  share?: number
}

/** A document in the one list: the chunk shown, the fused score, and where
 * each half put it. */
export interface Ranked extends Hit {
  words: Place | null
  meaning: Place | null
  /** The meaning alone found it, written in another language than the query. */
  across: boolean
}

/** Why a row was found: the sentence that answered, and the fragment that
 * shows it. */
export interface Explained {
  fragment: string
  closest: [number, number]
}

export interface Found {
  hits: Ranked[]
  /** Whether the hits the words found hold every word of the query. */
  all_words: boolean
  /** Whether the meaning answered too; `false` when the words answered alone. */
  by_meaning: boolean
  /** Documents the vector index does not cover yet. */
  behind: number
  took_ms: number
}

/** Why a file is not in the index, as the index's health sorts it. */
export type Why = 'not_a_document' | 'too_large' | 'locked' | 'unopened' | 'damaged'

/** A file found and not read - or a folder that could not be walked into. */
export interface Skipped {
  path: string
  why: Why
  /** Why, said for a person, in English: what the reader said. */
  reason: string
  /** In bytes, for a file. */
  size?: number
}

/** What the index lacks of the files under the folders, and what it holds of
 * them only in part. */
export interface Health {
  skipped: Skipped[]
  /** PDFs, Word documents and books with no text in them: found by name. */
  textless: string[]
  /** Why text in scans is not read on this machine, when it is not. */
  no_ocr?: string
  unrecognized: { path: string; reason: string }[]
  guessed: { path: string; encoding: string }[]
  /** Folders that cannot be reached now. */
  unavailable: string[]
}

/** What the window's indexing is doing. */
export type Indexing =
  | { kind: 'idle' }
  | { kind: 'running'; done: number; total: number }
  /** Held while the machine runs on its battery. */
  | { kind: 'held' }
  /** Another nooma is writing this library. */
  | { kind: 'busy' }
  | { kind: 'failed'; message: string }

export type IndexState = Indexing & {
  /** Updates finished since the window opened. */
  passes: number
  /** Whether the folders are watched for changes. */
  watching: boolean
  /** Counts every change: of two states, the larger is the newer. */
  seq: number
}

export interface Progress {
  done: number
  total: number
}

export interface MeaningStatus {
  model: string
  /** The model's size once fetched. */
  bytes: number
  present: boolean
  fetching: boolean
  /** Passages in the vector index; `null` before there is one. */
  passages: number | null
  behind: number
  computing: Progress | null
}

export interface VectorReport {
  model: string
  chunks: number
  passages: number
  embedded: number
  dropped: number
  rebuilt: string | null
  took_ms: number
  embed_ms: number
}

export interface VectorsDone {
  report: VectorReport | null
  /** `busy` when another nooma is computing the same vectors. */
  error: string | null
}

export interface FetchProgress {
  file: string
  done: number
  total: number
}

export const api = {
  status: () => invoke<Status>('status'),
  /** The words alone, ranked as the whole answer ranks them: fast. */
  search: (query: string) => invoke<Found>('search', { query }),
  /** The whole answer, words and meaning together; `null` when there is no
   * model or no vectors yet, and the words' answer is the whole one. */
  searchHybrid: (query: string) => invoke<Found | null>('search_hybrid', { query }),
  /** Documents that say what a pasted passage says. */
  similar: (text: string) => invoke<Found | null>('similar', { text }),
  /** The sentence of a row's chunk closest to the query, or to the pasted
   * passage; `null` when it cannot be told. */
  explain: (text: string, passage: boolean, path: string, line: number) =>
    invoke<Explained | null>('explain', { text, passage, path, line }),
  /** What the window's indexing is doing now; it says so as it changes by
   * `index-state` too. */
  indexState: () => invoke<IndexState>('index_state'),
  health: () => invoke<Health>('health'),
  meaningStatus: () => invoke<MeaningStatus>('meaning_status'),
  /** Starts computing vectors in the background; `false` when there is no model. */
  updateVectors: () => invoke<boolean>('update_vectors'),
  /** Starts fetching the model; `false` when a fetch is already running. */
  fetchModel: () => invoke<boolean>('fetch_model'),
  /** Adds the folder, and the window reads it. */
  addSource: (path: string) => invoke<Status>('add_source', { path }),
  open: (path: string) => invoke<void>('open_document', { path }),
  reveal: (path: string) => invoke<void>('reveal_document', { path }),
  onIndexState: (handler: (state: IndexState) => void): Promise<UnlistenFn> =>
    listen<IndexState>('index-state', (event) => handler(event.payload)),
  /** The vectors wait for the power: the machine runs on its battery. */
  onVectorsHeld: (handler: () => void): Promise<UnlistenFn> => listen('vectors-held', () => handler()),
  onVectorsProgress: (handler: (progress: Progress) => void): Promise<UnlistenFn> =>
    listen<Progress>('vectors-progress', (event) => handler(event.payload)),
  onVectorsDone: (handler: (done: VectorsDone) => void): Promise<UnlistenFn> =>
    listen<VectorsDone>('vectors-done', (event) => handler(event.payload)),
  onModelProgress: (handler: (progress: FetchProgress) => void): Promise<UnlistenFn> =>
    listen<FetchProgress>('model-progress', (event) => handler(event.payload)),
  /** Carries the error, or `null` when the model arrived whole. */
  onModelDone: (handler: (error: string | null) => void): Promise<UnlistenFn> =>
    listen<string | null>('model-done', (event) => handler(event.payload)),
}
