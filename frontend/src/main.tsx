import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './design/tokens.css'
import './design/base.css'
import './design/layout.css'
import './index.css'
import App from './App.tsx'

const root = createRoot(document.getElementById('root')!)

if (import.meta.env.DEV && location.pathname === '/__ui-gallery') {
  void import('./dev/UiGallery.tsx').then(({ UiGallery }) => root.render(<UiGallery />))
} else {
  root.render(<StrictMode><App /></StrictMode>)
}
