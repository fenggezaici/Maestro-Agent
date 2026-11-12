import { createContext, useContext, type ReactNode } from 'react';
import { labels, type UiPreferences } from './uiPreferences';

type Labels = ReturnType<typeof labels>;
const I18nContext = createContext<Labels>(labels('en'));

export function I18nProvider({ language, children }: { language: UiPreferences['language']; children: ReactNode }) {
  return <I18nContext.Provider value={labels(language)}>{children}</I18nContext.Provider>;
}

export function useT(): Labels {
  return useContext(I18nContext);
}
