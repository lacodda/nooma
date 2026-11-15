import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { overlayScrollbars } from 'dowel-ui'
import { App } from '@/App'
import '@/i18n'
import '@/styles.css'

const container = document.getElementById('root')
if (!container) {
  throw new Error('The #root element is missing from index.html')
}

// The engine's own overlay bars where it draws them, before anything scrolls.
overlayScrollbars()

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
