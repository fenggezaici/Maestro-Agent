import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { isTauri } from '@tauri-apps/api/core';
import { lazy, Suspense } from 'react';
import App from './App';
import '@fontsource/inter/latin-400.css';
import '@fontsource/inter/latin-500.css';
import '@fontsource/inter/latin-600.css';
import './styles/tokens.css';
import './index.css';
import './styles/phase1.css';
import './styles/phase2.css';
import './styles/phase3.css';

const PreviewApp = lazy(() => import('./preview/PreviewApp'));
const preview = !isTauri() && new URLSearchParams(window.location.search).get('preview') === '1';

createRoot(document.getElementById('root')!).render(
  <StrictMode><Suspense fallback={<div className="editor-loading">Loading workspace…</div>}>
    {preview ? <PreviewApp /> : <App />}
  </Suspense></StrictMode>,
);
