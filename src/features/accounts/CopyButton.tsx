import { IconButton, Tooltip } from '@mui/material';
import ContentCopyRoundedIcon from '@mui/icons-material/ContentCopyRounded';
import CheckRoundedIcon from '@mui/icons-material/CheckRounded';
import { writeText } from '@/platform/desktop';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';

export function CopyButton({ value, label }: { value: string; label: string }) {
  const { t } = useTranslation();
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle');
  const title =
    state === 'copied'
      ? t('feature.accountUi.copied')
      : state === 'failed'
        ? t('feature.accountUi.copyFailed')
        : t('feature.accountUi.copy', { label });
  return (
    <Tooltip title={title}>
      <IconButton
        size="small"
        aria-label={t('feature.accountUi.copy', { label })}
        color={state === 'failed' ? 'error' : 'default'}
        onClick={() => {
          void writeText(value)
            .then(() => setState('copied'))
            .catch(() => setState('failed'));
        }}
      >
        {state === 'copied' ? (
          <CheckRoundedIcon fontSize="small" />
        ) : (
          <ContentCopyRoundedIcon fontSize="small" />
        )}
      </IconButton>
    </Tooltip>
  );
}
