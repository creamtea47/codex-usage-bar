import i18n from '@/shared/i18n/i18n';
import { featureEn } from '@/shared/i18n/featureStrings';
/** 错误正文只允许固定错误码，不回显第三方异常。 */
export function accountError(error: unknown): string {
  const code = typeof error === 'string' ? error : '';
  return Object.hasOwn(featureEn.errors, code)
    ? i18n.t(`feature.errors.${code as keyof typeof featureEn.errors}`)
    : i18n.t('feature.failed');
}
