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
  kind: 'markdown' | 'text'
  title: string
  headings: string[]
  line: number
  fragment: string
  /** Byte offsets into the UTF-8 fragment, `[start, end]`. */
  highlights: [number, number][]
  tags: string[]
  score: number
}

export interface Found {
  hits: Hit[]
  took_ms: number
}

export interface UpdateReport {
  documents: number
  indexed: number
  removed: number
  chunks: number
  skipped: { path: string; reason: string }[]
  unavailable: string[]
  rebuilt: string | null
  took_ms: number
}

export interface Progress {
  done: number
  total: number
}

/** What a search by meaning returns; the score of a hit is a cosine. */
export interface MeaningFound {
  hits: Hit[]
  took_ms: number
  /** Documents the vector index does not cover yet. */
  behind: number
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
  search: (query: string) => invoke<Found>('search', { query }),
  /** `null` when there is no model or no vectors to search yet. */
  searchMeaning: (query: string) => invoke<MeaningFound | null>('search_meaning', { query }),
  /** Documents that say what a pasted passage says. */
  similar: (text: string) => invoke<MeaningFound | null>('similar', { text }),
  /** `null` when an update is already running in this window. */
  update: () => invoke<UpdateReport | null>('update'),
  meaningStatus: () => invoke<MeaningStatus>('meaning_status'),
  /** Starts computing vectors in the background; `false` when there is no model. */
  updateVectors: () => invoke<boolean>('update_vectors'),
  /** Starts fetching the model; `false` when a fetch is already running. */
  fetchModel: () => invoke<boolean>('fetch_model'),
  addSource: (path: string) => invoke<Status>('add_source', { path }),
  open: (path: string) => invoke<void>('open_document', { path }),
  reveal: (path: string) => invoke<void>('reveal_document', { path }),
  onProgress: (handler: (progress: Progress) => void): Promise<UnlistenFn> =>
    listen<Progress>('index-progress', (event) => handler(event.payload)),
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
