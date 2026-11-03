/*
 * A passage taken away as a quote: for a chat or a note, quoted as markdown
 * quotes, with where it came from under it - the file and the place in it -
 * so whoever reads it, the person later included, can find it again.
 *
 * The path is code, in backticks: a Windows path is full of backslashes, and
 * markdown reads `\_` as an escape. The quote keeps the passage's markdown as
 * it is written, which is what a note pasted into another note wants.
 */

/** Where the quoted text is in its file, said after the path. */
export interface Source {
  path: string
  /** The line, for a file a person opens in an editor. */
  line?: number
  /** The page, said in the window's language: `p. 4`. */
  page?: string
  /** The headings it is under, for a document with neither. */
  headings?: readonly string[]
}

/** Code in backticks, with enough of them that one inside does not end it. */
function code(text: string): string {
  const longest = Math.max(0, ...[...text.matchAll(/`+/g)].map((run) => run[0].length))
  const fence = '`'.repeat(longest + 1)
  return longest > 0 ? `${fence} ${text} ${fence}` : `${fence}${text}${fence}`
}

/** What a quote says it is from. */
export function sourceLine(source: Source): string {
  if (source.line !== undefined) return code(`${source.path}:${source.line}`)
  const after = source.page ?? (source.headings !== undefined && source.headings.length > 0 ? source.headings.join(' › ') : undefined)
  return after === undefined ? code(source.path) : `${code(source.path)} · ${after}`
}

/** A text quoted, and its source under it, inside the quote. */
export function quoteOf(text: string, source: Source): string {
  const lines = text.replace(/\r\n?/g, '\n').trim().split('\n')
  const quoted = lines.map((line) => (line.trim() === '' ? '>' : `> ${line}`))
  return [...quoted, '>', `> ${sourceLine(source)}`].join('\n') + '\n'
}
