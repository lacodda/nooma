/*
 * One field takes two kinds of input. A query is a few words typed; a passage
 * is a paragraph pasted from somewhere else, to find the documents that say
 * the same. The second is told from the first by how it arrives: pasted, and
 * either more than one line or longer than any query anyone types.
 */

/** A pasted text this long is a passage, even on one line. */
export const PASSAGE_CHARS = 140

/** Whether a pasted text is a passage to find more like, not a query. */
export function isPassage(pasted: string): boolean {
  const text = pasted.trim()
  return /[\r\n]/.test(text) || text.length >= PASSAGE_CHARS
}

/** A passage as one line, for the field to show. */
export function flatten(passage: string): string {
  return passage.replace(/\s+/g, ' ').trim()
}
