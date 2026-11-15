import type { Explained, Ranked } from '@/lib/api'

/*
 * Why a row was found, when its words were not why. A row the meaning found
 * with nothing marked in its fragment is asked which of its sentences
 * answered, as the core's `why::wants_why` decides it for the command line.
 * The list asks for every such row, top first, once the whole answer is in;
 * the preview asks for the row selected, so a row the list has not reached
 * yet is not waited for. Either way a row is asked once per search: the
 * model reading the same chunk twice would be time taken from the next row.
 */

/** Whether a row needs telling why it was found: the meaning found it,
 * nothing in its fragment is marked, and it has not been told yet. */
export function wantsWhy(hit: Ranked): boolean {
  return hit.meaning !== null && hit.highlights.length === 0 && hit.closest === undefined
}

/** Asks why a row was found, given its path and line. */
export type Ask = (path: string, line: number) => Promise<Explained | null>

/** A row's question asked once: the second asking of a row waits for the
 * first answer, and a failure is the same failure to every asker. */
export function askOnce(ask: Ask): (hit: Pick<Ranked, 'path' | 'line'>) => Promise<Explained | null> {
  const asked = new Map<string, Promise<Explained | null>>()
  return (hit) => {
    const key = `${hit.path}\n${hit.line}`
    let answer = asked.get(key)
    if (answer === undefined) {
      answer = ask(hit.path, hit.line)
      asked.set(key, answer)
    }
    return answer
  }
}
