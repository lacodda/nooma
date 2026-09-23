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

export const api = {
  status: () => invoke<Status>('status'),
  search: (query: string) => invoke<Found>('search', { query }),
  /** `null` when an update is already running in this window. */
  update: () => invoke<UpdateReport | null>('update'),
  addSource: (path: string) => invoke<Status>('add_source', { path }),
  open: (path: string) => invoke<void>('open_document', { path }),
  reveal: (path: string) => invoke<void>('reveal_document', { path }),
  onProgress: (handler: (progress: Progress) => void): Promise<UnlistenFn> =>
    listen<Progress>('index-progress', (event) => handler(event.payload)),
}
