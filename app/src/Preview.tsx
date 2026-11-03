import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type Ref } from 'react'
import { useTranslation } from 'react-i18next'
import { api, type Explained, type Passage, type Ranked, type Shown } from '@/lib/api'
import { type Mark, ordered, sentenceIn, utf16Ranges } from '@/lib/marks'
import { cn } from '@/lib/utils'
import { MARK_CLASSES, Note } from '@/Note'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Kbd } from '@/components/ui/kbd'
import { MarkedText } from '@/components/ui/marked-text'

/*
 * The row selected, read in its document: the chunk found, whole, among the
 * chunks before and after it, the query's words marked in all of them - and
 * the ways to take it further: open it at the place, show it in its folder,
 * take it away as a quote.
 *
 * A note is drawn as a note; a page of a PDF, a Word document or a book as
 * the text read out of it. Headings stand where the document's sections
 * start, and in a PDF where each page does. More of the document is a click
 * away above and below; the found chunk stays where the eye is while it
 * loads.
 *
 * A row the meaning found, with no word of the query in it, is asked which
 * of its sentences answered - once, when it has been selected a moment - and
 * that sentence is marked as the list marks one found in another language.
 */

/** Chunks shown on either side of the one found, and how many more a click
 * brings. */
const AROUND = 2
const STEP = 4

/** How long a row has to stay selected before it is read: walking the list
 * with the arrows reads only where it stops. */
const SETTLE_MS = 50

/** How long before a row the meaning found is asked why: the model is the
 * searches' first. */
const EXPLAIN_MS = 250

const KIND_LABELS: Record<Shown['kind'], string> = { markdown: 'MD', text: 'TXT', pdf: 'PDF', docx: 'DOCX', epub: 'EPUB', image: 'IMG' }

export type Copying = 'idle' | 'copied' | 'failed'

export interface PreviewProps {
  hit: Ranked
  /** The query typed, whose words are marked; `null` for a pasted passage. */
  query: string | null
  /** What the search was for, to ask a row found by meaning why. */
  asked: { text: string; passage: boolean }
  /** Moves whenever the index has changed: what is shown is read again. */
  stamp: number
  /** Told what is shown for a row - its path and line - so a quote can be
   * taken of it. */
  onShown: (path: string, line: number, shown: Shown | null) => void
  onQuote: () => void
  copying: Copying
}

interface Around {
  target: string
  before: number
  after: number
}

export function Preview({ hit, query, asked, stamp, onShown, onQuote, copying }: PreviewProps) {
  const { t } = useTranslation()
  const target = `${hit.path}\n${hit.line}\n${query ?? ''}`
  const [around, setAround] = useState<Around>({ target, before: AROUND, after: AROUND })
  // A new row starts with the chunks beside its own, not as far as the last
  // one was read.
  if (around.target !== target) setAround({ target, before: AROUND, after: AROUND })
  const [shown, setShown] = useState<{ target: string; shown: Shown | null } | null>(null)
  const [explained, setExplained] = useState<{ target: string; told: Explained } | null>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const scroller = useRef<HTMLDivElement>(null)
  /** Where the found chunk stood before more was read around it. */
  const anchor = useRef<number | null>(null)
  /** The row last read: reading more of it does not wait to settle. */
  const read = useRef<string | null>(null)
  /** The row the preview was last scrolled to. */
  const scrolledTo = useRef<string | null>(null)
  const { path, line } = hit

  useEffect(() => {
    let current = true
    const timer = setTimeout(
      () => {
        api
          .preview(path, line, query, around.before, around.after)
          .then((next) => {
            if (!current) return
            read.current = target
            setFailure(null)
            setShown({ target, shown: next })
            onShown(path, line, next)
          })
          .catch((error: unknown) => {
            if (current) setFailure(String(error))
          })
      },
      read.current === target ? 0 : SETTLE_MS,
    )
    return () => {
      current = false
      clearTimeout(timer)
    }
  }, [path, line, query, around.before, around.after, target, stamp, onShown])

  // A row the meaning alone found, with nothing marked in it: which of its
  // sentences answered.
  const askWhy = hit.meaning !== null && hit.highlights.length === 0 && hit.closest === undefined
  useEffect(() => {
    if (!askWhy) return
    let current = true
    const timer = setTimeout(() => {
      api
        .explain(asked.text, asked.passage, hit.path, hit.line)
        .then((told) => {
          if (current && told !== null && told !== undefined) setExplained({ target, told })
        })
        // The model failing on a sentence leaves the passage unmarked, which
        // is how it would be without asking.
        .catch(() => undefined)
    }, EXPLAIN_MS)
    return () => {
      current = false
      clearTimeout(timer)
    }
  }, [askWhy, asked.text, asked.passage, hit.path, hit.line, target])

  // What was read last stays until the row selected now is read: a blank
  // between two rows would flash at every arrow pressed.
  const current = shown?.shown ?? null
  const fresh = shown?.target === target

  // A new row puts its chunk near the top; more read around it keeps the
  // chunk where it was on the screen, and so does the same row read again.
  useLayoutEffect(() => {
    const box = scroller.current
    const found = box?.querySelector<HTMLElement>('[data-found]')
    if (!box || !found || shown === null) return
    if (anchor.current !== null) {
      box.scrollTop += found.offsetTop - anchor.current
      anchor.current = null
    } else if (scrolledTo.current !== shown.target) {
      // From the page and the headings it starts under, so the chunk is
      // seen where it is in the document.
      let top: HTMLElement = found
      while (top.previousElementSibling instanceof HTMLElement && top.previousElementSibling.hasAttribute('data-lead')) {
        top = top.previousElementSibling
      }
      box.scrollTop = Math.max(top.offsetTop - 12, 0)
    }
    scrolledTo.current = shown.target
  }, [shown])

  const more = (side: 'before' | 'after') => {
    const found = scroller.current?.querySelector<HTMLElement>('[data-found]')
    if (found) anchor.current = found.offsetTop
    setAround((now) => ({ ...now, [side]: now[side] + STEP }))
  }

  if (failure !== null) {
    return <Frame kind={hit.kind} path={hit.path} title={hit.title} body={<p className="text-xs text-bad">{t('preview.failed', { message: failure })}</p>} />
  }
  if (current === null) {
    return <Frame kind={hit.kind} path={hit.path} title={hit.title} body={fresh ? <p className="text-xs text-dim">{t('preview.missing')}</p> : null} />
  }

  // The sentence that answered by meaning: told with the row, or asked for.
  const why = !fresh ? null : hit.closest !== undefined ? { fragment: hit.fragment, closest: hit.closest } : explained?.target === target ? explained.told : null
  const foundPassage = current.passages[current.found]
  const closest = why !== null && foundPassage !== undefined ? sentenceIn(foundPassage.text, why.fragment, why.closest) : null
  const recognized = current.passages.some((passage) => passage.recognized === true)
  const opens = current.opens
  const openLabel =
    opens.at?.kind === 'line'
      ? t('preview.openAtLine', { line: opens.at.line })
      : opens.at?.kind === 'page'
        ? t('preview.openAtPage', { page: opens.at.page })
        : opens.at?.kind === 'heading'
          ? t('preview.openAtHeading')
          : t('preview.open')
  const openHint = opens.program !== null ? t('preview.opensIn', { program: opens.program }) : t('preview.opensInSystem')

  return (
    <Frame
      kind={current.kind}
      path={current.path}
      title={current.title}
      badge={recognized ? <Badge variant="outline" title={t('results.ocrHint')} className="shrink-0 rounded-xs px-1.5 py-0 font-mono text-2xs font-normal">{t('results.ocr')}</Badge> : null}
      scroller={scroller}
      body={
        <>
          {current.before > 0 && (
            <Button variant="link" size="xs" className="mb-2" onClick={() => more('before')}>
              {t('preview.earlier', { count: current.before })}
            </Button>
          )}
          {current.passages.map((passage, index) => (
            <PassageView
              key={`${passage.line}:${index}`}
              passage={passage}
              previous={current.passages[index - 1]}
              title={current.title}
              kind={current.kind}
              found={index === current.found}
              closest={index === current.found ? closest : null}
            />
          ))}
          {current.after > 0 && (
            <Button variant="link" size="xs" className="mt-1" onClick={() => more('after')}>
              {t('preview.later', { count: current.after })}
            </Button>
          )}
        </>
      }
      foot={
        <>
          <Button variant="primary" size="sm" title={openHint} onClick={() => void api.open(hit.path, hit.line)}>
            <span className="truncate">{openLabel}</span>
            <Kbd keys="Enter" className="max-md:hidden [&_kbd]:border-on-accent/40 [&_kbd]:bg-transparent [&_kbd]:text-on-accent" />
          </Button>
          <Button size="sm" onClick={() => void api.reveal(hit.path)}>
            {t('preview.reveal')}
            <Kbd keys="Mod+Enter" className="max-lg:hidden" />
          </Button>
          <Button size="sm" onClick={onQuote}>
            {copying === 'copied' ? t('preview.copied') : copying === 'failed' ? t('preview.copyFailed') : t('preview.quote')}
            <Kbd keys="Mod+Shift+C" className="max-lg:hidden" />
          </Button>
        </>
      }
    />
  )
}

function Frame({
  kind,
  path,
  title,
  badge,
  scroller,
  body,
  foot,
}: {
  kind: Shown['kind']
  path: string
  title: string
  badge?: ReactNode
  scroller?: Ref<HTMLDivElement>
  body: ReactNode
  foot?: ReactNode
}) {
  const { t } = useTranslation()
  return (
    <section data-preview aria-label={t('preview.label')} className="flex h-full min-h-0 flex-col bg-raise">
      <header className="shrink-0 border-b border-line px-4 pt-2.5 pb-2">
        <div className="flex min-w-0 items-center gap-2">
          <span className="shrink-0 rounded-xs border border-line px-1 font-mono text-2xs font-bold text-dim">{KIND_LABELS[kind]}</span>
          <h2 className="truncate text-sm font-semibold">{title}</h2>
          {badge}
        </div>
        <p className="mt-0.5 truncate font-mono text-2xs text-faint" title={path}>
          {path}
        </p>
      </header>
      <div ref={scroller} className="relative min-h-0 flex-1 overflow-y-auto px-4 py-3">
        {body}
      </div>
      {foot !== undefined && <footer className="flex shrink-0 flex-wrap items-center gap-2 border-t border-line px-4 py-2">{foot}</footer>}
    </section>
  )
}

const HEADING_CLASSES = ['text-base font-semibold', 'text-sm font-semibold', 'text-sm font-semibold text-dim']

/** How many headings two paths share from the top. */
function shared(a: readonly string[], b: readonly string[]): number {
  let at = 0
  while (at < a.length && at < b.length && a[at] === b[at]) at++
  return at
}

function PassageView({
  passage,
  previous,
  title,
  kind,
  found,
  closest,
}: {
  passage: Passage
  previous: Passage | undefined
  title: string
  kind: Shown['kind']
  found: boolean
  closest: [number, number] | null
}) {
  const { t } = useTranslation()
  // The headings that start here: the ones the chunk before was not under. A
  // note's top heading is its title, said in the header already.
  const from = previous === undefined ? 0 : shared(previous.headings, passage.headings)
  const headings = passage.headings.map((heading, depth) => ({ heading, depth })).slice(from).filter(({ heading, depth }) => !(depth === 0 && heading === title))
  const newPage = kind === 'pdf' && passage.page !== undefined && passage.page !== previous?.page
  const marks: Mark[] = ordered([
    ...utf16Ranges(passage.text, passage.highlights).map(([start, end]) => ({ start, end, kind: 'word' as const })),
    ...(closest === null ? [] : [{ start: closest[0], end: closest[1], kind: 'closest' as const }]),
  ])
  return (
    <>
      {newPage && (
        <p data-lead className="mt-3 mb-1 font-mono text-2xs text-faint first:mt-0" aria-label={t('results.page', { page: passage.page })}>
          {t('results.page', { page: passage.page })}
        </p>
      )}
      {headings.map(({ heading, depth }) => (
        <p key={depth} data-lead role="heading" aria-level={Math.min(depth + 3, 6)} className={cn('mt-3 mb-1 first:mt-0', HEADING_CLASSES[Math.min(depth, 2)])}>
          {heading}
        </p>
      ))}
      <div
        data-found={found ? '' : undefined}
        className={cn(
          '-ml-3 mb-2 border-l-2 pl-2.5',
          // The found chunk is a shade lighter than the sentence marked in it.
          found ? 'rounded-r-md border-accent bg-accent-soft/50 py-2 pr-3 text-text' : 'border-line py-0.5 text-dim',
        )}
      >
        {passage.text === '' ? null : kind === 'markdown' ? (
          <Note text={passage.text} marks={marks} />
        ) : (
          <MarkedText
            text={passage.text}
            marks={marks.map((mark) => ({ start: mark.start, end: mark.end, className: MARK_CLASSES[mark.kind] }))}
            className="text-sm leading-relaxed"
          />
        )}
      </div>
    </>
  )
}
