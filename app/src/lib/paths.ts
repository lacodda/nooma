import type { Source } from '@/lib/api'

const BACKSLASH = String.fromCharCode(92)

/**
 * Where a file is, said the short way: its folder inside the source, led by
 * the source's own name - `Notes/home` rather than the whole absolute path,
 * which on Windows fills the row before it reaches anything that differs
 * between two results. The full path stays available as the row's tooltip.
 */
export function folderOf(path: string, sources: readonly Source[]): string {
  const separator = path.includes(BACKSLASH) ? BACKSLASH : '/'
  const trim = (root: string) => (root.endsWith(separator) ? root.slice(0, -1) : root)
  const source = sources
    .map((s) => trim(s.path))
    .filter((root) => path.startsWith(root + separator))
    .sort((a, b) => b.length - a.length)[0]
  const folder = path.slice(0, Math.max(path.lastIndexOf(separator), 0))
  if (source === undefined) return folder
  const name = source.slice(source.lastIndexOf(separator) + 1)
  return name + folder.slice(source.length)
}
