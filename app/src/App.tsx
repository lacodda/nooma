import { useCallback, useEffect, useRef, useState, type ClipboardEvent, type KeyboardEvent, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { useLocale } from 'dowel-ui'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { open as chooseFolder } from '@tauri-apps/plugin-dialog'
import { api, type Found, type MeaningStatus, type Ranked, type Source, type Status } from '@/lib/api'
import { splitHighlights } from '@/lib/highlight'
import { flatten, isPassage } from '@/lib/passage'
import { folderOf } from '@/lib/paths'
import { cn } from '@/lib/utils'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { EmptyState } from '@/components/ui/empty-state'
import { Progress } from '@/components/ui/progress'
import { SearchField } from '@/components/ui/search-field'
import { StatusDot } from '@/components/ui/status-dot'
import { ResizeEdges, WindowButtons, useTitleBarGestures } from '@/components/ui/window-frame'

/*
 * The whole window: a bar with the field in it, the results, and a status
 * line. The field lives in the title bar rather than under it - the search is
 * the product, and a strip above it that only holds the window's buttons
 * would be the system title bar the line removed, drawn again by hand.
 *
 * There is no splash. The window is spotlight-shaped - summoned, typed into,
 * dismissed - and anything between opening it and typing is in the way. The
 * index is brought up to date in the background while the field already
 * answers from what is stored.
 *
 * Results are one list, ranked from both halves: the documents that hold the
 * words and the documents close in meaning, each once. The words answer in
 * milliseconds; the whole answer waits for the model to read the query - tens
 * of milliseconds, or a batch of the background reading when that is running.
 * The words' answer is held back a moment, so that a whole answer arriving
 * soon is drawn alone rather than reordering the list under the eye, and
 * stands in when it does not.
 */

type Indexing =
  | { kind: 'idle' }
  | { kind: 'running'; done: number; total: number }
  | { kind: 'busy' }
  | { kind: 'failed'; message: string }

/** Computing vectors in the background. `started` and `from` date the run,
 * so the time left can be told from its pace. */
type Vectors =
  | { kind: 'idle' }
  | { kind: 'running'; done: number; total: number; started: number; from: number }
  | { kind: 'busy' }
  | { kind: 'failed'; message: string }

type Fetching = { kind: 'idle' } | { kind: 'running'; done: number; total: number } | { kind: 'failed'; message: string }

const inTauri = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/** How long typing has to pause before the query is sent. Short: the exact
 * index answers in milliseconds, and a slower field feels like a slower one. */
const DEBOUNCE_MS = 60

/** The time left is not told before a run has this long a pace behind it. */
const PACE_MS = 20_000

/** How long the words' answer waits for the whole one before it is shown. */
const GRACE_MS = 120

export function App() {
  const { t } = useTranslation()
  const [status, setStatus] = useState<Status | null>(null)
  const [indexing, setIndexing] = useState<Indexing>({ kind: 'idle' })
  const [query, setQuery] = useState('')
  /** The pasted passage being searched for, when the field holds one. */
  const [example, setExample] = useState<string | null>(null)
  /** The answer the list shows: the words' alone, or the whole one. */
  const [found, setFound] = useState<Found | null>(null)
  /** The whole answer is still coming. */
  const [pending, setPending] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const [meaningFailure, setMeaningFailure] = useState<string | null>(null)
  const [meaningStatus, setMeaningStatus] = useState<MeaningStatus | null>(null)
  const [vectors, setVectors] = useState<Vectors>({ kind: 'idle' })
  const [fetching, setFetching] = useState<Fetching>({ kind: 'idle' })
  /** Bumped when the vectors change, so the shown results follow them. */
  const [vectorsVersion, setVectorsVersion] = useState(0)
  const [selected, setSelected] = useState(0)
  /** The document selected, so the selection stays on it when the whole
   * answer replaces the words'. */
  const selectedPath = useRef<string | null>(null)
  const asked = useRef(0)

  const select = useCallback((index: number, hits: readonly Ranked[]) => {
    setSelected(index)
    selectedPath.current = hits[index]?.path ?? null
  }, [])

  const runSearch = useCallback(
    (text: string, passage: string | null) => {
      const ticket = ++asked.current
      // An older query answering after a newer one must not overwrite it.
      const current = () => ticket === asked.current
      if (passage === null && text.trim() === '') {
        setFound(null)
        setPending(false)
        setFailure(null)
        setMeaningFailure(null)
        return
      }
      let shownOnce = false
      const show = (next: Found) => {
        setFound(next)
        setFailure(null)
        // A new query starts at the top; the whole answer to the same query
        // keeps the document that was selected, wherever it moved.
        const kept = shownOnce ? next.hits.findIndex((hit) => hit.path === selectedPath.current) : -1
        select(Math.max(kept, 0), next.hits)
        shownOnce = true
      }
      let whole: 'coming' | 'none' | 'shown' = 'coming'
      let words: Found | null = null
      const started = Date.now()
      setPending(true)
      if (passage !== null) {
        // A paragraph's words are not a query: every one of them would have
        // to occur, and the words would answer with noise.
        setFound(null)
        setFailure(null)
      }
      const both = passage === null ? api.searchHybrid(text) : api.similar(passage)
      both
        .then((result) => {
          if (!current()) return
          setPending(false)
          setMeaningFailure(null)
          if (result !== null) {
            whole = 'shown'
            show(result)
          } else {
            whole = 'none'
            if (words !== null) show(words)
          }
        })
        .catch((error: unknown) => {
          if (!current()) return
          whole = 'none'
          setPending(false)
          setMeaningFailure(String(error))
          if (words !== null) show(words)
        })
      if (passage !== null) return
      api
        .search(text)
        .then((result) => {
          if (!current() || whole === 'shown') return
          words = result
          if (whole === 'none') {
            show(result)
            return
          }
          const wait = Math.max(0, GRACE_MS - (Date.now() - started))
          setTimeout(() => {
            if (current() && whole === 'coming') show(result)
          }, wait)
        })
        .catch((error: unknown) => {
          if (current()) setFailure(String(error))
        })
    },
    [select],
  )

  const readMeaningStatus = useCallback(async () => {
    const current = await api.meaningStatus()
    setMeaningStatus(current ?? null)
    return current ?? null
  }, [])

  /** Catch the vectors up with the index, in the background, once there is
   * a model to compute them with. */
  const computeVectors = useCallback(async () => {
    const current = await readMeaningStatus()
    if (current?.present) await api.updateVectors()
  }, [readMeaningStatus])

  const refresh = useCallback(async () => {
    setIndexing({ kind: 'running', done: 0, total: 0 })
    const stop = await api.onProgress(({ done, total }) => setIndexing({ kind: 'running', done, total }))
    try {
      await api.update()
      setIndexing({ kind: 'idle' })
    } catch (error) {
      setIndexing(String(error) === 'busy' ? { kind: 'busy' } : { kind: 'failed', message: String(error) })
    } finally {
      stop()
    }
    setStatus(await api.status())
    await computeVectors()
  }, [computeVectors])

  // The background work reports as it goes, for as long as the window is
  // open: the vectors being computed, and the model being fetched.
  useEffect(() => {
    if (!inTauri()) return
    const stops = [
      api.onVectorsProgress(({ done, total }) =>
        setVectors((previous) =>
          previous.kind === 'running' && previous.total === total
            ? { ...previous, done }
            : { kind: 'running', done, total, started: Date.now(), from: done },
        ),
      ),
      api.onVectorsDone(({ error }) => {
        setVectors(error === null ? { kind: 'idle' } : error === 'busy' ? { kind: 'busy' } : { kind: 'failed', message: error })
        setVectorsVersion((version) => version + 1)
        void readMeaningStatus()
      }),
      api.onModelProgress(({ done, total }) => setFetching({ kind: 'running', done, total })),
      api.onModelDone((error) => {
        setFetching(error === null ? { kind: 'idle' } : { kind: 'failed', message: error })
        if (error === null) void computeVectors()
        else void readMeaningStatus()
      }),
    ]
    return () => {
      for (const stop of stops) void stop.then((unlisten) => unlisten())
    }
  }, [computeVectors, readMeaningStatus])

  // Open: show the window once there is something in it, then catch the
  // index up with the folders while the field is already usable.
  useEffect(() => {
    if (inTauri()) void getCurrentWindow().show()
    api
      .status()
      .then((current) => {
        setStatus(current)
        if (current.sources.length > 0) void refresh()
        else void readMeaningStatus()
      })
      .catch((error: unknown) => setIndexing({ kind: 'failed', message: String(error) }))
  }, [refresh, readMeaningStatus])

  // Results follow the query, and follow the index and the vectors as they
  // fill.
  useEffect(() => {
    const timer = setTimeout(() => runSearch(query, example), DEBOUNCE_MS)
    return () => clearTimeout(timer)
  }, [query, example, status, vectorsVersion, runSearch])

  const addFolder = useCallback(async () => {
    const folder = await chooseFolder({ directory: true, title: t('sources.choose') })
    if (typeof folder !== 'string') return
    try {
      setStatus(await api.addSource(folder))
      await refresh()
    } catch (error) {
      setIndexing({ kind: 'failed', message: String(error) })
    }
  }, [refresh, t])

  const fetchModel = useCallback(async () => {
    setFetching({ kind: 'running', done: 0, total: meaningStatus?.bytes ?? 0 })
    try {
      await api.fetchModel()
    } catch (error) {
      setFetching({ kind: 'failed', message: String(error) })
    }
  }, [meaningStatus])

  const onQuery = useCallback((value: string) => {
    setExample(null)
    setQuery(value)
  }, [])

  const onPaste = (event: ClipboardEvent<HTMLInputElement>) => {
    const pasted = event.clipboardData.getData('text')
    if (!isPassage(pasted)) return
    event.preventDefault()
    setExample(pasted)
    setQuery(flatten(pasted))
  }

  const rows = found?.hits ?? []
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      select(Math.min(selected + 1, Math.max(rows.length - 1, 0)), rows)
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      select(Math.max(selected - 1, 0), rows)
    } else if (event.key === 'Enter') {
      const row = rows[selected]
      if (!row) return
      event.preventDefault()
      void (event.ctrlKey || event.metaKey ? api.reveal(row.path) : api.open(row.path))
    }
  }

  return (
    <div className="flex h-full flex-col bg-bg text-text">
      <ResizeEdges />
      <Titlebar query={query} onQuery={onQuery} onKeyDown={onKeyDown} onPaste={onPaste} />
      <main className="min-h-0 flex-1 overflow-y-auto">
        <Body
          status={status}
          query={query}
          example={example}
          found={found}
          pending={pending}
          failure={failure}
          meaningFailure={meaningFailure}
          meaningStatus={meaningStatus}
          vectors={vectors}
          fetching={fetching}
          selected={selected}
          onSelect={(index) => select(index, rows)}
          onAddFolder={addFolder}
          onFetchModel={fetchModel}
        />
      </main>
      <StatusBar
        status={status}
        indexing={indexing}
        found={found}
        meaningStatus={meaningStatus}
        vectors={vectors}
        fetching={fetching}
        onAddFolder={addFolder}
      />
    </div>
  )
}

function Titlebar({
  query,
  onQuery,
  onKeyDown,
  onPaste,
}: {
  query: string
  onQuery: (value: string) => void
  onKeyDown: (event: KeyboardEvent<HTMLInputElement>) => void
  onPaste: (event: ClipboardEvent<HTMLInputElement>) => void
}) {
  const { t } = useTranslation()
  const gestures = useTitleBarGestures()
  return (
    <header className="flex h-titlebar shrink-0 items-center gap-3 border-b border-line bg-raise pl-3" {...gestures}>
      <Mark />
      <SearchField
        value={query}
        onValueChange={onQuery}
        onKeyDown={onKeyDown}
        onPaste={onPaste}
        aria-label={t('search.label')}
        placeholder={t('search.placeholder')}
        clearLabel={t('search.clear')}
        shortcut={['Mod', 'K']}
        autoFocus
        spellCheck={false}
        className="w-full max-w-2xl"
      />
      <WindowButtons
        className="ml-auto"
        labels={{
          minimize: t('window.minimize'),
          maximize: t('window.maximize'),
          restore: t('window.restore'),
          close: t('window.close'),
        }}
      />
    </header>
  )
}

/** The product's mark, small: the hexagon and its code, in the accent. */
function Mark() {
  return (
    <svg viewBox="0 0 100 100" className="size-5 shrink-0 text-accent" aria-hidden>
      <polygon
        points="50,5 89,27.5 89,72.5 50,95 11,72.5 11,27.5"
        fill="currentColor"
        stroke="currentColor"
        strokeWidth="9"
        strokeLinejoin="round"
      />
      <text
        x="50"
        y="66"
        textAnchor="middle"
        className="fill-on-accent font-mono"
        fontWeight="800"
        fontSize="46"
      >
        nm
      </text>
    </svg>
  )
}

/** Megabytes for a person, as the CLI says them. */
function megabytes(bytes: number): string {
  return String(Math.round(bytes / 1_000_000))
}

function Body({
  status,
  query,
  example,
  found,
  pending,
  failure,
  meaningFailure,
  meaningStatus,
  vectors,
  fetching,
  selected,
  onSelect,
  onAddFolder,
  onFetchModel,
}: {
  status: Status | null
  query: string
  example: string | null
  found: Found | null
  pending: boolean
  failure: string | null
  meaningFailure: string | null
  meaningStatus: MeaningStatus | null
  vectors: Vectors
  fetching: Fetching
  selected: number
  onSelect: (index: number) => void
  onAddFolder: () => void
  onFetchModel: () => void
}) {
  const { t } = useTranslation()
  if (status === null) return null
  if (status.sources.length === 0) {
    return (
      <EmptyState
        className="h-full"
        title={t('sources.none')}
        body={t('sources.noneBody')}
        action={
          <Button variant="primary" onClick={onAddFolder}>
            {t('sources.add')}
          </Button>
        }
      />
    )
  }
  if (failure !== null) {
    return <EmptyState className="h-full" variant="error" title={t('results.failed')} body={failure} />
  }
  if (example === null && query.trim() === '') {
    return <EmptyState className="h-full" title={t('results.type')} body={t('results.typeBody')} />
  }
  // The words' answer is held back a moment for the whole one: nothing to
  // draw yet, and an empty frame for a few milliseconds reads as nothing.
  if (example === null && found === null) return null

  const hits = found?.hits ?? []
  const byMeaning = found?.by_meaning ?? false
  // Nothing from either half: the whole window says so, with what to try.
  if (example === null && byMeaning && hits.length === 0) {
    return <EmptyState className="h-full" variant="filtered" title={t('results.nothing')} body={t('results.nothingBody')} />
  }
  // What the half by meaning could not do, once it is known that it could
  // not: an offer of the model above the list, or what is still missing.
  const notice =
    !byMeaning && !pending ? (
      <MeaningNotice
        failure={meaningFailure}
        meaningStatus={meaningStatus}
        vectors={vectors}
        fetching={fetching}
        actionOnly={hits.length > 0}
        onFetchModel={onFetchModel}
      />
    ) : null
  const notes = [
    ...(found !== null && !found.all_words && hits.some((hit) => hit.words !== null) ? [t('results.partial')] : []),
    ...(found !== null && found.behind > 0 ? [t('results.behind', { count: found.behind })] : []),
  ]
  const best = hits[0]?.score ?? 1
  return (
    <div className="py-1.5">
      {notice}
      <SectionHeading
        title={example !== null ? t('results.similarSection') : t('results.found', { count: hits.length })}
        note={notes.length > 0 ? notes.join(' · ') : undefined}
      />
      {found !== null && hits.length === 0 && <Quiet>{example !== null ? t('results.nothingSimilar') : t('results.nothingWords')}</Quiet>}
      <ul aria-label={t('results.label')} className="pb-1.5">
        {hits.map((hit, index) => (
          <ResultRow
            key={hit.path}
            hit={hit}
            allWords={found?.all_words ?? true}
            sources={status.sources}
            relevance={best > 0 ? hit.score / best : 0}
            selected={index === selected}
            onSelect={() => onSelect(index)}
          />
        ))}
      </ul>
    </div>
  )
}

function SectionHeading({ title, note }: { title: string; note?: string }) {
  return (
    <h2 className="flex items-baseline gap-2 px-4 pt-2.5 pb-1 font-mono text-2xs tracking-wider text-faint uppercase">
      <span>{title}</span>
      {note !== undefined && <span className="tracking-normal normal-case">· {note}</span>}
    </h2>
  )
}

function Quiet({ children }: { children: ReactNode }) {
  return <p className="px-4 pb-2.5 text-xs text-dim">{children}</p>
}

/** What the half by meaning says when it did not answer: why, and what would
 * change that. Above a list, only what asks for an action - the model to
 * fetch, a failure - is said: the status line tells the rest. */
function MeaningNotice({
  failure,
  meaningStatus,
  vectors,
  fetching,
  actionOnly,
  onFetchModel,
}: {
  failure: string | null
  meaningStatus: MeaningStatus | null
  vectors: Vectors
  fetching: Fetching
  actionOnly: boolean
  onFetchModel: () => void
}) {
  const { t, i18n } = useTranslation()
  const locale = useLocale(i18n.language)
  if (meaningStatus === null) return null
  if (failure !== null) return <Quiet>{t('meaning.failed', { message: failure })}</Quiet>

  const box = (children: ReactNode) => <div className="mx-4 mb-2.5 rounded-md border border-line bg-raise p-3">{children}</div>

  if (!meaningStatus.present) {
    if (fetching.kind === 'running') {
      return box(
        <Progress label={t('meaning.downloading')} value={fetching.total > 0 ? fetching.done : null} max={Math.max(fetching.total, 1)} size="sm">
          {t('meaning.downloadingOf', { done: megabytes(fetching.done), total: megabytes(fetching.total) })}
        </Progress>,
      )
    }
    return box(
      <div className="flex flex-col gap-2">
        <p className="text-sm font-semibold">{t('meaning.needsModel')}</p>
        <p className="text-xs leading-relaxed text-dim">{t('meaning.needsModelBody', { size: megabytes(meaningStatus.bytes) })}</p>
        {fetching.kind === 'failed' && <p className="text-xs text-bad">{t('meaning.downloadFailed', { message: fetching.message })}</p>}
        <div>
          <Button variant="primary" size="sm" onClick={onFetchModel}>
            {fetching.kind === 'failed' ? t('meaning.retry') : t('meaning.download')}
          </Button>
        </div>
      </div>,
    )
  }
  if (actionOnly) return null
  if (vectors.kind === 'running') {
    return box(
      <Progress label={t('meaning.reading')} value={vectors.total > 0 ? vectors.done : null} max={Math.max(vectors.total, 1)} size="sm">
        {t('meaning.readingOf', { done: vectors.done.toLocaleString(locale), total: vectors.total.toLocaleString(locale) })}
      </Progress>,
    )
  }
  return <Quiet>{t('meaning.notYet')}</Quiet>
}

function ResultRow({
  hit,
  allWords,
  sources,
  relevance,
  selected,
  onSelect,
}: {
  hit: Ranked
  allWords: boolean
  sources: readonly Source[]
  relevance: number
  selected: boolean
  onSelect: () => void
}) {
  const { t } = useTranslation()
  const element = useRef<HTMLLIElement>(null)
  useEffect(() => {
    if (selected) element.current?.scrollIntoView?.({ block: 'nearest' })
  }, [selected])

  // A note's top heading is usually its title; saying it twice is noise.
  const headings = hit.headings[0] === hit.title ? hit.headings.slice(1) : hit.headings
  const where = [folderOf(hit.path, sources), ...(headings.length > 0 ? [headings.join(' › ')] : []), t('results.line', { line: hit.line })]
  const badge = 'shrink-0 rounded-xs px-1.5 py-0 font-mono text-2xs font-normal'
  return (
    <li
      ref={element}
      aria-selected={selected}
      onClick={onSelect}
      onDoubleClick={() => void api.open(hit.path)}
      className={cn(
        'cursor-default border-l-2 px-4 py-2.5',
        selected ? 'border-accent bg-accent-soft' : 'border-transparent hover:bg-soft',
      )}
    >
      <div className="flex min-w-0 items-center gap-2">
        <span className="shrink-0 rounded-xs border border-line px-1 font-mono text-2xs font-bold text-dim">
          {hit.kind === 'markdown' ? 'MD' : 'TXT'}
        </span>
        <span className="truncate text-sm font-semibold">{hit.title}</span>
        {/* Which halves found it. The cosine is in the meaning's hint: it
          * means the same from one search to the next, the bar does not. */}
        {hit.words !== null && (
          <Badge variant="info" title={allWords ? t('results.wordsHint') : t('results.someWordsHint')} className={badge}>
            {allWords ? t('results.words') : t('results.someWords')}
          </Badge>
        )}
        {hit.meaning !== null && (
          <Badge variant="accent" title={t('results.meaningHint', { score: hit.meaning.score.toFixed(2) })} className={badge}>
            {t('results.meaning')}
          </Badge>
        )}
        <span className="ml-auto flex shrink-0 items-center" aria-hidden>
          <span className="h-0.5 w-11 overflow-hidden rounded-full bg-line">
            <span className="block h-full rounded-full bg-accent" style={{ width: `${Math.round(relevance * 100)}%` }} />
          </span>
        </span>
      </div>
      <div className="mt-0.5 truncate font-mono text-2xs text-faint" title={hit.path}>
        {where.join(' · ')}
      </div>
      {hit.fragment !== '' && (
        <p className="mt-1 line-clamp-2 text-xs leading-relaxed text-dim">
          {splitHighlights(hit.fragment, hit.highlights).map((piece, index) =>
            piece.marked ? (
              <mark key={index} className="border-b border-info bg-info-soft px-px text-text">
                {piece.text}
              </mark>
            ) : (
              <span key={index}>{piece.text}</span>
            ),
          )}
        </p>
      )}
    </li>
  )
}

/** What the status line says about the meaning half, and in which tone. */
function meaningState(
  meaningStatus: MeaningStatus | null,
  vectors: Vectors,
  fetching: Fetching,
  t: (key: string, options?: Record<string, unknown>) => string,
  locale: string,
): { tone: 'good' | 'warn' | 'bad' | 'info' | 'neutral'; label: string; title?: string } | null {
  if (meaningStatus === null) return null
  if (fetching.kind === 'running') {
    const percent = fetching.total > 0 ? Math.floor((fetching.done / fetching.total) * 100) : 0
    return { tone: 'info', label: t('status.meaningFetching', { percent }) }
  }
  if (!meaningStatus.present) return { tone: 'neutral', label: t('status.meaningNoModel') }
  if (vectors.kind === 'running') {
    const label = t('status.meaningReading', { done: vectors.done.toLocaleString(locale), total: vectors.total.toLocaleString(locale) })
    const elapsed = Date.now() - vectors.started
    const read = vectors.done - vectors.from
    if (elapsed < PACE_MS || read <= 0) return { tone: 'warn', label }
    const minutes = Math.max(1, Math.ceil(((elapsed / read) * (vectors.total - vectors.done)) / 60_000))
    return { tone: 'warn', label: `${label} · ${t('status.meaningLeft', { minutes })}` }
  }
  if (vectors.kind === 'busy') return { tone: 'info', label: t('status.meaningBusy') }
  if (vectors.kind === 'failed') return { tone: 'bad', label: t('status.meaningFailed'), title: vectors.message }
  if (meaningStatus.passages === null) return { tone: 'neutral', label: t('status.meaningNone') }
  if (meaningStatus.behind > 0) return { tone: 'warn', label: t('status.meaningBehind', { count: meaningStatus.behind }) }
  return { tone: 'good', label: t('status.meaningReady') }
}

function StatusBar({
  status,
  indexing,
  found,
  meaningStatus,
  vectors,
  fetching,
  onAddFolder,
}: {
  status: Status | null
  indexing: Indexing
  found: Found | null
  meaningStatus: MeaningStatus | null
  vectors: Vectors
  fetching: Fetching
  onAddFolder: () => void
}) {
  const { t, i18n } = useTranslation()
  const locale = useLocale(i18n.language)
  if (status === null) return <footer className="h-7 shrink-0 border-t border-line bg-raise" />

  const state =
    indexing.kind === 'running'
      ? {
          tone: 'warn' as const,
          label: indexing.total > 0 ? t('status.indexing', { done: indexing.done, total: indexing.total }) : t('status.starting'),
        }
      : indexing.kind === 'busy'
        ? { tone: 'info' as const, label: t('status.busy') }
        : indexing.kind === 'failed'
          ? { tone: 'bad' as const, label: t('status.failed') }
          : status.stale !== null
            ? { tone: 'warn' as const, label: t('status.stale') }
            : { tone: 'good' as const, label: t('status.current') }
  const half = status.sources.length > 0 ? meaningState(meaningStatus, vectors, fetching, t, locale) : null

  return (
    <footer
      className="flex h-7 shrink-0 items-center gap-4 border-t border-line bg-raise px-4 font-mono text-2xs whitespace-nowrap text-faint"
      title={indexing.kind === 'failed' ? indexing.message : undefined}
    >
      {status.sources.length > 0 && <StatusDot className="shrink-0" status={state.tone} label={state.label} showLabel />}
      {half !== null && (
        <span className="shrink-0" title={half.title}>
          <StatusDot status={half.tone} label={half.label} showLabel />
        </span>
      )}
      {/* The counts give way first when the window is narrow: the states
        * beside them are what the line is for. */}
      {status.sources.length > 0 && (
        <span className="min-w-0 truncate">
          {t('status.documents', { count: status.documents })} · {t('status.sources', { count: status.sources.length })}
        </span>
      )}
      {status.sources.length > 0 && (
        <Button variant="icon" size="sm" className="h-5 shrink-0 px-1.5 font-mono text-2xs" onClick={onAddFolder}>
          + {t('sources.addShort')}
        </Button>
      )}
      <span className="ml-auto shrink-0">{found !== null ? t('status.took', { ms: found.took_ms }) : t('status.offline')}</span>
    </footer>
  )
}
