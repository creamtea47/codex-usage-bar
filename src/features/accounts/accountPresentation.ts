import type { Account } from '@/contracts/accounts';
export function accountTitle(account: Account): string {
  return account.details?.email ?? account.details?.name ?? account.label;
}

/** 自动生成的旧标签仍保存在配置中，只在展示上避免与身份信息重复。 */
export function accountNote(account: Account): string | null {
  const email = account.details?.email;
  const masked = email ? `${email[0]}***@${email.split('@').at(-1)}` : null;
  return [email, masked, `Account ${account.id.slice(0, 8)}`].includes(account.label) ? null : account.label;
}

export function recordDate(value: string | null | undefined, language: string): string {
  if (!value || !Number.isFinite(Date.parse(value))) return '—';
  return new Intl.DateTimeFormat(language, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(new Date(value));
}
