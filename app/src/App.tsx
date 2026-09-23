import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { open as chooseFolder } from '@tauri-apps/plugin-dialog'
import { api, type Found, type Hit, type Source, type Status } from '@/lib/api'
import { splitHighlights } from '@/lib/highlight'
import { folderOf } from '@/lib/paths'
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import { EmptyState } from '@/components/ui/empty-state'
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
 */

type Indexing =
  | { kind: 'idle' }
  | { kind: 'running'; done: number; total: number }
  | { kind: 'busy' }
  | { kind: 'failed'; message: string }

const inTauri = () => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/** How long typing has to pause before the query is sent. Short: the exact
 * index answers in milliseconds, and a slower field feels like a slower one. */
const DEBOUNCE_MS = 60

export function App() {
  const { t } = useTranslation()
  const [status, setStatus] = useState<Status | null>(null)
  const [indexing, setIndexing] = useState<Indexing>({ kind: 'idle' })
  const [query, setQuery] = useState('')
  const [found, setFound] = useState<Found | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const [selected, setSelected] = useState(0)
  const asked = useRef(0)

  const runSearch = useCallback((text: string) => {
    const ticket = ++asked.current
    if (text.trim() === '') {
      setFound(null)
      setFailure(null)
      return
    }
    api
      .search(text)
      .then((result) => {
        // An older query answering after a newer one must not overwrite it.
        if (ticket !== asked.current) return
        setFound(result)
        setFailure(null)
        setSelected(0)
      })
      .catch((error: unknown) => {
        if (ticket !== asked.current) return
        setFailure(String(error))
      })
  }, [])

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
  }, [])

  // Open: show the window once there is something in it, then catch the
  // index up with the folders while the field is already usable.
  useEffect(() => {
    if (inTauri()) void getCurrentWindow().show()
    api
      .status()
      .then((current) => {
        setStatus(current)
        if (current.sources.length > 0) void refresh()
      })
      .catch((error: unknown) => setIndexing({ kind: 'failed', message: String(error) }))
  }, [refresh])

  // Results follow the query, and follow the index as it fills.
  useEffect(() => {
    const timer = setTimeout(() => runSearch(query), DEBOUNCE_MS)
    return () => clearTimeout(timer)
  }, [query, status, runSearch])

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

  const hits = found?.hits ?? []
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      setSelected((index) => Math.min(index + 1, Math.max(hits.length - 1, 0)))
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      setSelected((index) => Math.max(index - 1, 0))
    } else if (event.key === 'Enter') {
      const hit = hits[selected]
      if (!hit) return
      event.preventDefault()
      void (event.ctrlKey || event.metaKey ? api.reveal(hit.path) : api.open(hit.path))
    }
  }

  return (
    <div className="flex h-full flex-col bg-bg text-text">
      <ResizeEdges />
      <Titlebar query={query} onQuery={setQuery} onKeyDown={onKeyDown} />
      <main className="min-h-0 flex-1 overflow-y-auto">
        <Body
          status={status}
          query={query}
          found={found}
          failure={failure}
          selected={selected}
          onSelect={setSelected}
          onAddFolder={addFolder}
        />
      </main>
      <StatusBar status={status} indexing={indexing} found={found} onAddFolder={addFolder} />
    </div>
  )
}

function Titlebar({
  query,
  onQuery,
  onKeyDown,
}: {
  query: string
  onQuery: (value: string) => void
  onKeyDown: (event: KeyboardEvent<HTMLInputElement>) => void
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

function Body({
  status,
  query,
  found,
  failure,
  selected,
  onSelect,
  onAddFolder,
}: {
  status: Status | null
  query: string
  found: Found | null
  failure: string | null
  selected: number
  onSelect: (index: number) => void
  onAddFolder: () => void
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
  if (query.trim() === '' || found === null) {
    return <EmptyState className="h-full" title={t('results.type')} body={t('results.typeBody')} />
  }
  if (found.hits.length === 0) {
    return <EmptyState className="h-full" variant="filtered" title={t('results.nothing')} body={t('results.nothingBody')} />
  }
  const best = found.hits[0]?.score ?? 1
  return (
    <ul aria-label={t('results.label')} className="py-1.5">
      {found.hits.map((hit, index) => (
        <ResultRow
          key={hit.path}
          hit={hit}
          sources={status.sources}
          relevance={best > 0 ? hit.score / best : 0}
          selected={index === selected}
          onSelect={() => onSelect(index)}
        />
      ))}
    </ul>
  )
}

function ResultRow({
  hit,
  sources,
  relevance,
  selected,
  onSelect,
}: {
  hit: Hit
  sources: readonly Source[]
  relevance: number
  selected: boolean
  onSelect: () => void
}) {
  const { t } = useTranslation()
  const row = useRef<HTMLLIElement>(null)
  useEffect(() => {
    if (selected) row.current?.scrollIntoView?.({ block: 'nearest' })
  }, [selected])

  // A note's top heading is usually its title; saying it twice is noise.
  const headings = hit.headings[0] === hit.title ? hit.headings.slice(1) : hit.headings
  const where = [folderOf(hit.path, sources), ...(headings.length > 0 ? [headings.join(' › ')] : []), t('results.line', { line: hit.line })]
  return (
    <li
      ref={row}
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
        <span
          title={t('results.exactHint')}
          className="shrink-0 rounded-xs bg-info-soft px-1.5 font-mono text-2xs text-info"
        >
          {t('results.exact')}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5" aria-hidden>
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

function StatusBar({
  status,
  indexing,
  found,
  onAddFolder,
}: {
  status: Status | null
  indexing: Indexing
  found: Found | null
  onAddFolder: () => void
}) {
  const { t } = useTranslation()
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

  return (
    <footer
      className="flex h-7 shrink-0 items-center gap-4 border-t border-line bg-raise px-4 font-mono text-2xs text-faint"
      title={indexing.kind === 'failed' ? indexing.message : undefined}
    >
      {status.sources.length > 0 && <StatusDot status={state.tone} label={state.label} showLabel />}
      {status.sources.length > 0 && (
        <span>
          {t('status.documents', { count: status.documents })} · {t('status.sources', { count: status.sources.length })}
        </span>
      )}
      {status.sources.length > 0 && (
        <Button variant="icon" size="sm" className="h-5 px-1.5 font-mono text-2xs" onClick={onAddFolder}>
          + {t('sources.addShort')}
        </Button>
      )}
      <span className="ml-auto">{found !== null ? t('status.took', { ms: found.took_ms }) : t('status.offline')}</span>
    </footer>
  )
}
