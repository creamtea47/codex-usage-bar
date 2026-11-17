import { Typography } from '@mui/material';
import { useTranslation } from 'react-i18next';
import type { ExtraCredits } from '@/contracts/usage';

export function ExtraCreditBalance({ credits }: { credits?: ExtraCredits | null }) {
  const { t } = useTranslation();
  if (!credits) return null;
  const value = credits.unlimited
    ? t('feature.accountUi.unlimited')
    : (credits.balance ??
      (credits.hasCredits == null
        ? null
        : t(credits.hasCredits ? 'feature.accountUi.available' : 'feature.accountUi.unavailable')));
  return value == null ? null : (
    <Typography variant="body2">
      {t('feature.accountUi.extraCredits')} · {value}
    </Typography>
  );
}
