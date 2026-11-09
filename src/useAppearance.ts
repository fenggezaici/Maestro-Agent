import { useLayoutEffect, useState } from 'react';
import type { UiPreferences } from './uiPreferences';

export function useAppearance(preferences: UiPreferences) {
  const [resolved, setResolved] = useState<'dark' | 'light'>(preferences.theme === 'light' ? 'light' : 'dark');
  useLayoutEffect(() => {
    const query = typeof window.matchMedia === 'function' ? window.matchMedia('(prefers-color-scheme: dark)') : null;
    const apply = () => {
      const theme = preferences.theme === 'system' ? query?.matches ? 'dark' : 'light' : preferences.theme;
      document.documentElement.dataset.theme = theme;
      document.documentElement.style.colorScheme = theme;
      setResolved(theme);
      document.documentElement.lang = preferences.language;
      document.documentElement.style.setProperty('--code-font', `"${preferences.editorFont}", Consolas, monospace`);
      document.documentElement.style.setProperty('--code-size', `${preferences.editorFontSize}px`);
      window.dispatchEvent(new CustomEvent('maestro-theme', { detail: theme }));
    };
    apply();
    query?.addEventListener('change', apply);
    return () => query?.removeEventListener('change', apply);
  }, [preferences.theme, preferences.language, preferences.editorFont, preferences.editorFontSize]);
  return resolved;
}
