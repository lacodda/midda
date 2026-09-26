import { useEffect } from 'react'

import { ProductMark } from '@/components/ui/product-mark'
import { Splash as SplashScreen } from '@/components/ui/splash'
import { useTitleBarGestures } from '@/components/ui/window-frame'
import { SPLASH_MARK_SIZE } from '@/splash-page'
import words from '@/splash.json'

/**
 * What the window shows while it is opening.
 *
 * The same picture `index.html` painted before the bundle arrived - the mark,
 * the name, the promise, the version, what it is doing and one thing worth
 * knowing - drawn with the same words from the same file, so nothing moves
 * when this takes over. The static one is removed here, on the first render,
 * because from this moment the page is the one drawing.
 *
 * For midda the wait is short and usually a blink: the volumes report how full
 * they are at once. It is a disconnected network drive that makes it long,
 * and that is when a window saying what it is doing beats a blank one.
 *
 * The whole splash is a handle, the way the title bar is once it arrives: the
 * window has no system frame, and a window waiting on a slow drive should
 * still move out of the way.
 */
export function Splash() {
  const gestures = useTitleBarGestures()

  useEffect(() => {
    document.getElementById('splash')?.remove()
  }, [])

  return (
    <SplashScreen
      {...gestures}
      mark={<ProductMark product="midda" size={SPLASH_MARK_SIZE} />}
      name="midda"
      tagline={words.tagline}
      version={`v${__APP_VERSION__}`}
      status={words.status}
      tip={words.tip}
    />
  )
}
