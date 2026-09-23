import { describe, expect, it } from 'vitest'
import { folderOf } from '@/lib/paths'

const B = String.fromCharCode(92)
const win = (...parts: string[]) => parts.join(B)

describe('folderOf', () => {
  it('names the folder inside its source, led by the source', () => {
    const sources = [{ path: win('C:', 'Users', 'me', 'Notes') }]
    expect(folderOf(win('C:', 'Users', 'me', 'Notes', 'home', 'a.md'), sources)).toBe(win('Notes', 'home'))
    expect(folderOf(win('C:', 'Users', 'me', 'Notes', 'a.md'), sources)).toBe('Notes')
  })

  it('takes the longest matching source, and not a sibling with a shared prefix', () => {
    const sources = [{ path: '/data/notes' }, { path: '/data/notes-old' }]
    expect(folderOf('/data/notes-old/x/a.md', sources)).toBe('notes-old/x')
  })

  it('falls back to the whole folder outside every source', () => {
    expect(folderOf('/elsewhere/a.md', [{ path: '/data' }])).toBe('/elsewhere')
  })
})
