import { describe, expect, it } from 'vitest'
import type { Blockquote } from 'mdast'
import { calloutOf, inlinesOf, parseNote, toneOf } from '@/lib/note'

const pieces = (value: string) =>
  inlinesOf(value).map((inline) => {
    const text = value.slice(inline.start, inline.end)
    return inline.kind === 'link' ? `${inline.embed ? 'embed' : 'link'}:${inline.shown}` : `${inline.kind}:${text}`
  })

describe('inlinesOf', () => {
  it('reads links between notes as Obsidian shows them', () => {
    expect(pieces('see [[Seeds]], [[Seeds#Tomatoes]], [[#Vents]] and [[Seeds|the seed list]]')).toEqual([
      'text:see ',
      'link:Seeds',
      'text:, ',
      'link:Seeds > Tomatoes',
      'text:, ',
      'link:Vents',
      'text: and ',
      'link:the seed list',
    ])
    expect(pieces('![[greenhouse.png]]')).toEqual(['embed:greenhouse.png'])
  })

  it('reads tags, but not a number or a heading link', () => {
    expect(pieces('#garden and (#seeds/tomato) but not #1 or a#b')).toEqual([
      'tag:#garden',
      'text: and (',
      'tag:#seeds/tomato',
      'text:) but not #1 or a#b',
    ])
    expect(pieces('#теплица весной')).toEqual(['tag:#теплица', 'text: весной'])
  })

  it('reads highlights, and hides comments', () => {
    const value = 'water ==every evening== %%not at noon%% daily'
    const inlines = inlinesOf(value)
    expect(pieces(value)).toEqual(['text:water ', 'highlight:==every evening==', 'text: ', 'comment:%%not at noon%%', 'text: daily'])
    const highlight = inlines.find((inline) => inline.kind === 'highlight')
    expect(highlight?.kind === 'highlight' && value.slice(...highlight.inner)).toBe('every evening')
  })

  it('leaves plain text whole', () => {
    expect(pieces('nothing here = a == b')).toEqual(['text:nothing here = a == b'])
  })
})

describe('calloutOf', () => {
  const quote = (text: string) => parseNote(text).children[0] as Blockquote

  it('reads a callout, its kind and its title', () => {
    const callout = calloutOf(quote('> [!Warning] Frost tonight\n> Close the vents.'))
    expect(callout?.type).toBe('warning')
    expect(callout?.title).toBe('Frost tonight')
    const body = callout?.body?.children[0]
    expect(body?.type === 'text' && body.value).toBe('Close the vents.')
    // The body starts where its line does in the passage, the `> ` of the
    // quote before it: the letters are placed from there.
    expect(body?.position?.start.offset).toBe('> [!Warning] Frost tonight\n'.length)
  })

  it('reads a callout with no title or body', () => {
    expect(calloutOf(quote('> [!tip]'))).toEqual({ type: 'tip', title: null, body: null })
  })

  it('leaves a plain quote a quote', () => {
    expect(calloutOf(quote('> A quote [!not] a callout'))).toBeNull()
  })

  it('draws each kind in the tone Obsidian groups it in', () => {
    expect(['note', 'tip', 'warning', 'danger', 'quote', 'unknown'].map(toneOf)).toEqual(['info', 'good', 'warn', 'bad', 'neutral', 'info'])
  })
})
