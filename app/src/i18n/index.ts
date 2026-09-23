import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'
import en from './locales/en.json'
import ru from './locales/ru.json'

// English is the source language, never a fallback for a missing translation.
// `tools/check-locales.mjs` holds every locale to the same shape at build time,
// so there is nothing to fall back *to*: a gap fails the build instead of
// reaching a person as a stray English line in another language's window.
export const defaultNS = 'translation'
export const resources = {
  en: { translation: en },
  ru: { translation: ru },
} as const

/** The window speaks the system's language when it has it, English otherwise. */
export function systemLanguage(languages: readonly string[] = navigator.languages): keyof typeof resources {
  for (const tag of languages) {
    const base = tag.toLowerCase().split('-')[0]
    if (base === 'ru' || base === 'en') return base
  }
  return 'en'
}

void i18n.use(initReactI18next).init({
  resources,
  lng: systemLanguage(),
  fallbackLng: false,
  interpolation: { escapeValue: false },
})

export default i18n
