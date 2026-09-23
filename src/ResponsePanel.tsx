import { Alert, Box, Paper, Stack, Typography } from '@mui/material';
import { useTranslation } from 'react-i18next';
import { formatDateTime } from './format';
import type { QuotaAutoContinueResult } from './types';

/** 回复按纯文本展示，不执行模型输出中的 HTML、链接或指令。 */
export function ResponsePanel({ result, automatic }: { result: QuotaAutoContinueResult | null; automatic: boolean }) {
  const { t, i18n } = useTranslation();
  const detail = result?.details;
  return <Paper variant="outlined" sx={{ p: 2 }}><Stack spacing={1}>
    <Typography variant="subtitle1" sx={{ fontWeight: 700 }}>{t(automatic ? 'feature.automaticReply' : 'feature.manualReply')}</Typography>
    {result ? <>
      <Typography variant="caption">{formatDateTime(detail?.startedAt ?? result.attemptedAt, i18n.language)} · {t('feature.requestedModel')}: {result.model ?? '—'}{detail && ` · HTTP ${detail.httpStatus ?? '—'} · ${detail.durationMs} ms`}</Typography>
      {detail?.responseModel && <Typography variant="caption">{t('feature.responseModel')}: {detail.responseModel}</Typography>}
      {result.errorCode && <Alert severity="error">{t(`settings.quotaAutoContinue.errors.${result.errorCode}`)}</Alert>}
      {detail?.errorMessage && <Typography variant="body2" sx={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>{detail.errorMessage}</Typography>}
      {detail?.deliveryUncertain && <Alert severity="warning">{t('feature.uncertain')}</Alert>}
      <Box component="pre" sx={{ m: 0, p: 1.25, bgcolor: 'action.hover', borderRadius: 1, maxHeight: 260, overflow: 'auto', whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', font: 'inherit', fontSize: 13 }}>
        {detail ? detail.text || (result.successAt ? t('feature.noText') : '—') : t('feature.legacyReply')}
      </Box>
      {detail?.truncated && <Typography variant="caption">{t('feature.truncated')}</Typography>}
    </> : <Typography color="text.secondary">{t('feature.noResult')}</Typography>}
  </Stack></Paper>;
}
