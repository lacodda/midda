import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { markLevel, marks } from 'dowel-ui/marks'
import palette from 'dowel-ui/palettes/midda.json' with { type: 'json' }

import { fillSplashPage, SPLASH_MARK_SIZE, splashValues } from '@/splash-page'
import words from '@/splash.json'

// Vitest runs from the repository root. Not `import.meta.url`: under jsdom it
// resolved `../index.html` to the root of the drive.
const read = (path: string) => readFileSync(join(process.cwd(), path), 'utf8')

describe('the splash in index.html', () => {
  const page = fillSplashPage(read('index.html'), splashValues('9.8.7'))

  it('is filled in completely', () => {
    // A brace left in the page is drawn in the window, in the product's name.
    expect(page).not.toContain('{{')
    expect(page).toContain('v9.8.7')
  })

  it('says what the React half says, so nothing jumps when it takes over', () => {
    // An apostrophe is prose, not markup, and stays as it is.
    for (const line of [words.tagline, words.status, words.tip]) expect(page).toContain(line)
  })

  it('draws the mark the React half draws at the same size', () => {
    // Both come from the line's masters: this one written into the page, the
    // other drawn by ProductMark at SPLASH_MARK_SIZE. A hand-drawn copy here
    // is what kilna has, and nothing holds it to the master.
    expect(markLevel(SPLASH_MARK_SIZE)).toBe('M')
    expect(page).toContain(marks.midda.M)
  })

  it('paints with the theme midda is drawn in, in both themes', () => {
    for (const theme of ['dark', 'light'] as const) {
      expect(page).toContain(`background: ${palette.color[theme].bg.$value};`)
      expect(page).toContain(`--splash-accent: ${palette.color[theme].accent.$value};`)
    }
  })
})

describe('filling the page', () => {
  const values = { version: '1.0.0', tip: 'a tip' }

  it('refuses a name it does not know', () => {
    expect(() => fillSplashPage('{{version}} {{tip}} {{tagline}}', values)).toThrow(/tagline/)
  })

  it('refuses a page that leaves a value out', () => {
    // A line the React half draws and this one does not is the jump the
    // shared file exists to prevent.
    expect(() => fillSplashPage('{{version}}', values)).toThrow(/tip/)
  })

  it('refuses a brace it cannot read as a name', () => {
    expect(() => fillSplashPage('{{version}} {{tip}} {{ version }}', values)).toThrow(/not a name/)
  })
})

describe('the window before the page', () => {
  it('opens on the dark background the splash paints', () => {
    // The window exists before the page does, and shows this colour until
    // the page is shown. The value is written in tauri.conf.json, which
    // cannot import the palette, so it is held to it here.
    const config = JSON.parse(read('src-tauri/tauri.conf.json')) as {
      app: { windows: { backgroundColor: string; decorations: boolean; visible: boolean }[] }
    }
    const [window] = config.app.windows
    expect(window?.backgroundColor).toBe(palette.color.dark.bg.$value)
    // Frameless, because the page draws the title bar, and hidden until the
    // page has painted its splash.
    expect(window?.decorations).toBe(false)
    expect(window?.visible).toBe(false)
  })
})
