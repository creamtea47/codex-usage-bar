import { en } from './en';
import { zhCN } from './zhCN';
import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';

export type LanguagePreference = 'system' | 'zh-CN' | 'en';
export type SupportedLanguage = Exclude<LanguagePreference, 'system'>;

export const resources = {
  'zh-CN': zhCN,
  en: en,
} as const;

declare module 'i18next' {
  interface CustomTypeOptions {
    defaultNS: 'translation';
    resources: (typeof resources)['en'];
  }
}

export function resolveSupportedLanguage(language?: string | null): SupportedLanguage {
  return language && /^zh(?:-|$)/i.test(language) ? 'zh-CN' : 'en';
}

export function systemLanguage(): SupportedLanguage {
  if (typeof navigator === 'undefined') return 'en';
  const language = navigator.languages?.find(Boolean) ?? navigator.language;
  return resolveSupportedLanguage(language);
}

export function resolveLanguagePreference(
  preference: LanguagePreference,
  detectedSystemLanguage: string | null = null,
): SupportedLanguage {
  if (preference === 'zh-CN' || preference === 'en') return preference;
  return detectedSystemLanguage === null
    ? systemLanguage()
    : resolveSupportedLanguage(detectedSystemLanguage);
}

export function currentLanguage(): SupportedLanguage {
  return resolveSupportedLanguage(i18n.resolvedLanguage ?? i18n.language);
}

export async function applyLanguagePreference(preference: LanguagePreference): Promise<SupportedLanguage> {
  const language = resolveLanguagePreference(preference);
  if (i18n.resolvedLanguage !== language) await i18n.changeLanguage(language);
  if (typeof document !== 'undefined') document.documentElement.lang = language;
  return language;
}

void i18n.use(initReactI18next).init({
  resources,
  lng: systemLanguage(),
  fallbackLng: 'en',
  supportedLngs: ['zh-CN', 'en'],
  defaultNS: 'translation',
  interpolation: { escapeValue: false },
  initAsync: false,
  returnNull: false,
});

if (typeof document !== 'undefined') document.documentElement.lang = currentLanguage();

export default i18n;
