import { useTranslation } from 'react-i18next'
import { api, type Health, type Source, type Why } from '@/lib/api'
import { folderOf } from '@/lib/paths'
import { Button } from '@/components/ui/button'
import { Drawer, DrawerBody, DrawerClose, DrawerDescription, DrawerHeader, DrawerPopup, DrawerTitle } from '@/components/ui/drawer'
import { ListRow, RowButton } from '@/components/ui/list-row'

/*
 * The index's health: what it lacks of the files under the folders, and what
 * it holds of them only in part - files too large, under a password, damaged,
 * scans with no text, encodings guessed, folders out of reach.
 *
 * It answers the question a person has when a document they know is there is
 * not found, so it is grouped by what is wrong rather than by folder, the
 * groups a person can do something about first. A row shows the file in its
 * folder; the reader's own words - in English, and technical - are the row's
 * tooltip, so a translated window says nothing half in another language.
 */

/** How many files and folders the panel has something to say about. */
export function healthCount(health: Health): number {
  return health.skipped.length + health.textless.length + health.unrecognized.length + health.guessed.length + health.unavailable.length
}

/** The order the reasons are said in: what a person can fix first. */
const WHYS: readonly Why[] = ['locked', 'damaged', 'too_large', 'unopened', 'not_a_document']

const BACKSLASH = String.fromCharCode(92)

function nameOf(path: string): string {
  return path.slice(Math.max(path.lastIndexOf('/'), path.lastIndexOf(BACKSLASH)) + 1)
}

/** Megabytes as the reason says them: the limits are in binary ones. */
function megabytes(bytes: number): number {
  return Math.round(bytes / 1024 / 1024)
}

interface Row {
  path: string
  /** What is measured about it: a size, an encoding. */
  end?: string
  /** The reader's own words. */
  detail?: string
  /** A file, which can be shown in its folder; a folder out of reach cannot. */
  file: boolean
}

interface Group {
  key: string
  title: string
  hint: string
  rows: Row[]
}

export function HealthPanel({
  health,
  sources,
  open,
  onOpenChange,
}: {
  health: Health
  sources: readonly Source[]
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const groups: Group[] = [
    ...WHYS.map((why) => ({
      key: why,
      title: t(`health.why.${why}`),
      hint: t(`health.whyHint.${why}`),
      rows: health.skipped
        .filter((skipped) => skipped.why === why)
        .map((skipped) => ({
          path: skipped.path,
          end: why === 'too_large' && skipped.size !== undefined ? t('health.megabytes', { size: megabytes(skipped.size) }) : undefined,
          detail: skipped.reason,
          // A folder the walk could not go into has no size.
          file: skipped.size !== undefined,
        })),
    })),
    {
      key: 'textless',
      title: t('health.textless'),
      hint: health.no_ocr !== undefined ? t('health.textlessNoOcr') : t('health.textlessHint'),
      rows: health.textless.map((path) => ({ path, detail: health.no_ocr, file: true })),
    },
    {
      key: 'unrecognized',
      title: t('health.unrecognized'),
      hint: t('health.unrecognizedHint'),
      rows: health.unrecognized.map(({ path, reason }) => ({ path, detail: reason, file: true })),
    },
    {
      key: 'guessed',
      title: t('health.guessed'),
      hint: t('health.guessedHint'),
      rows: health.guessed.map(({ path, encoding }) => ({ path, end: encoding, file: true })),
    },
    {
      key: 'unavailable',
      title: t('health.unavailable'),
      hint: t('health.unavailableHint'),
      rows: health.unavailable.map((path) => ({ path, file: false })),
    },
  ].filter((group) => group.rows.length > 0)

  return (
    <Drawer open={open} onOpenChange={onOpenChange} swipeDirection="right">
      <DrawerPopup side="right" size="lg">
        <DrawerHeader
          action={
            <DrawerClose render={<Button variant="icon" size="icon-sm" aria-label={t('health.close')} />}>
              <svg viewBox="0 0 16 16" aria-hidden fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
                <path d="M4 4l8 8M12 4l-8 8" />
              </svg>
            </DrawerClose>
          }
        >
          <DrawerTitle>{t('health.title')}</DrawerTitle>
          <DrawerDescription>{t('health.body', { count: healthCount(health) })}</DrawerDescription>
        </DrawerHeader>
        <DrawerBody>
          {groups.length === 0 && <p className="pb-5 text-sm text-dim">{t('health.clear')}</p>}
          {groups.map((group) => (
            <section key={group.key} aria-labelledby={`health-${group.key}`} className="pb-5">
              <h3 id={`health-${group.key}`} className="font-mono text-2xs tracking-wider text-faint uppercase">
                {group.title} · {group.rows.length}
              </h3>
              <p className="mt-0.5 mb-1.5 text-xs leading-relaxed text-dim">{group.hint}</p>
              <ul className="-mx-2.5">
                {group.rows.map((row) => (
                  <li key={row.path} title={row.detail === undefined ? row.path : `${row.path}\n${row.detail}`}>
                    {row.file ? (
                      <RowButton
                        description={folderOf(row.path, sources)}
                        end={row.end}
                        aria-label={t('health.reveal', { name: nameOf(row.path) })}
                        onClick={() => void api.reveal(row.path)}
                      >
                        {nameOf(row.path)}
                      </RowButton>
                    ) : (
                      <ListRow className="border-b-0 px-2.5">{row.path}</ListRow>
                    )}
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </DrawerBody>
      </DrawerPopup>
    </Drawer>
  )
}
