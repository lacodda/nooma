import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { Mark } from '@/lib/marks'
import { Note, NoteLine } from '@/Note'

/** Marks on every place a word stands in a text. */
function marksOf(text: string, word: string): Mark[] {
  const out: Mark[] = []
  for (let at = text.indexOf(word); at >= 0; at = text.indexOf(word, at + word.length)) {
    out.push({ start: at, end: at + word.length, kind: 'word' })
  }
  return out
}

function drawn(text: string, word: string) {
  const { container } = render(<Note text={text} marks={marksOf(text, word)} />)
  return { container, marked: [...container.querySelectorAll('mark')].map((mark) => mark.textContent) }
}

describe('Note', () => {
  it('marks a word wherever the note puts it', () => {
    const text = [
      '## Vents at noon',
      '',
      '> A quote that runs',
      '> on to vents',
      '',
      '- a list item',
      '  that goes on to vents',
      '- [ ] check the vents',
      '',
      '| when | what |',
      '| --- | --- |',
      '| noon | vents |',
      '',
      '```',
      'open(vents)',
      '```',
      '',
      '> [!tip] Vents',
      '> Open the vents wide.',
    ].join('\n')
    const { marked } = drawn(text, 'vents')
    expect(marked).toHaveLength(6)
    expect(marked.every((word) => word === 'vents')).toBe(true)
  })

  it('marks a word after an escape, and not across an entity it cannot place', () => {
    expect(drawn('a \\*vents\\* note', 'vents').marked).toEqual(['vents'])
    expect(drawn('fish &amp; vents', 'vents').marked).toEqual([])
  })

  it('draws a link as a link that goes nowhere, and raw HTML as text', () => {
    const { container } = drawn('[the site](https://example.com) and <script>alert(1)</script> ![a plant](plant.png)', 'none')
    expect(container.querySelector('a')).toBeNull()
    expect(container.querySelector('script')).toBeNull()
    expect(container.querySelector('img')).toBeNull()
    expect(container.textContent).toContain('the site')
    expect(container.textContent).toContain('<script>alert(1)</script>')
    expect(container.querySelector('[title="https://example.com"]')?.textContent).toBe('the site')
    expect(container.textContent).toContain('a plant')
  })

  it('draws a callout with its title in the tone of its kind', () => {
    const { container } = drawn('> [!warning]\n> Frost tonight.', 'Frost')
    const callout = container.querySelector('[data-callout="warning"]') as HTMLElement
    expect(callout.className).toContain('border-warn')
    expect(callout.textContent).toBe('WarningFrost tonight.')
  })

  it('marks a link inside the sentence that answered as the sentence, not as a word', () => {
    const text = 'See also [[kitchen-appliances]] for the kettle.'
    const marks: Mark[] = [{ start: 0, end: text.length, kind: 'closest' }]
    const { container } = render(<Note text={text} marks={marks} />)
    const link = container.querySelector('[title="kitchen-appliances"]') as HTMLElement
    expect(link.className).toContain('bg-accent-soft')
    expect(link.className).not.toContain('bg-info-soft')
  })
})

describe('NoteLine', () => {
  it('runs the blocks of a note into one line, set apart by a dot', () => {
    const text = [
      '## Frost',
      '',
      'Shut the vents.',
      '',
      '- water',
      '- [x] sow',
      '',
      '| plant | when |',
      '| --- | --- |',
      '| tomato | morning |',
      '',
      '---',
      '',
      '[spec]: https://example.org',
    ].join('\n')
    const { container } = render(<NoteLine text={text} marks={[]} />)
    expect(container.textContent).toBe('Frost · Shut the vents. · water · sow · plant · when · tomato · morning')
    expect(container.querySelector('br')).toBeNull()
  })

  it('reads a hard break as a space, and marks the words where they stand', () => {
    const text = 'Below five  \ndegrees'
    const { container } = render(<NoteLine text={text} marks={marksOf(text, 'degrees')} />)
    expect(container.textContent).toBe('Below five degrees')
    expect([...container.querySelectorAll('mark')].map((mark) => mark.textContent)).toEqual(['degrees'])
  })
})
