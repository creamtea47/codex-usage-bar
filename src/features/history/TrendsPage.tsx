import DeleteOutlineRoundedIcon from '@mui/icons-material/DeleteOutlineRounded';
import QueryStatsRoundedIcon from '@mui/icons-material/QueryStatsRounded';
import StorageRoundedIcon from '@mui/icons-material/StorageRounded';
import {
  Alert,
  Box,
  Button,
  Card,
  CardContent,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  LinearProgress,
  Snackbar,
  Stack,
  Switch,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
  useTheme,
  type AlertColor,
} from '@mui/material';
import { AdapterDayjs } from '@mui/x-date-pickers/AdapterDayjs';
import { DesktopDatePicker } from '@mui/x-date-pickers/DesktopDatePicker';
import { LocalizationProvider } from '@mui/x-date-pickers/LocalizationProvider';
import { enUS, zhCN } from '@mui/x-date-pickers/locales';
import type { DateValidationError, PickerChangeHandlerContext } from '@mui/x-date-pickers/models';
import dayjs, { type Dayjs } from 'dayjs';
import 'dayjs/locale/zh-cn';
import { useCallback, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { formatDateTime } from '@/shared/format';
import {
  buildCustomHistoryRequest,
  defaultCustomHistoryDates,
  hasPartialHistoryCoverage,
  validateCustomHistoryDates,
  type CustomHistoryDateIssue,
} from './historyRange';
import type { UsageHistoryPreset, UsageHistoryRequest } from '@/contracts/history';

import { languageForFormatting, fallbackWindowLabel, type TrendsTranslationKey } from './chartData';
import { useHistoryQuery, type HistoryQuerySource } from './useHistoryQuery';
import { SeriesCard } from './SeriesCard';
export interface TrendsPageProps extends HistoryQuerySource {
  accountId?: string | null;
  historyEnabled: boolean;
  onSetHistoryEnabled: (enabled: boolean) => Promise<void>;
  onClearUsageHistory: () => Promise<void>;
  /** Injectable clock keeps local-day boundaries deterministic in tests. */
  now?: () => Date;
}
interface Feedback {
  id: number;
  key: TrendsTranslationKey;
  severity: AlertColor;
}

type RangeChoice = UsageHistoryPreset | 'custom';

function pickerIssue(error: DateValidationError): CustomHistoryDateIssue | null {
  if (error === null) return null;
  if (error === 'disableFuture' || error === 'maxDate') return 'future';
  return 'invalidEnd';
}

function customIssueKey(issue: CustomHistoryDateIssue): TrendsTranslationKey {
  if (issue === 'future') return 'trends.custom.error.future';
  if (issue === 'reversed') return 'trends.custom.error.reversed';
  if (issue === 'emptyToday') return 'trends.custom.error.emptyToday';
  if (issue === 'missingStart' || issue === 'missingEnd') return 'trends.custom.error.required';
  return 'trends.custom.error.invalid';
}

/** Lazy-load this default export from SettingsWindow so Recharts stays out of the main-card chunk. */
export default function TrendsPage({
  accountId = null,
  historyEnabled,
  getUsageHistory,
  onSetHistoryEnabled,
  onClearUsageHistory,
  listenForUsageHistory,
  now = () => new Date(),
}: TrendsPageProps) {
  const { t, i18n } = useTranslation();
  const theme = useTheme();
  const language = languageForFormatting(i18n.resolvedLanguage ?? i18n.language);
  const translate = useCallback(
    (key: TrendsTranslationKey, options?: Record<string, unknown>) =>
      t(key as never, options as never) as unknown as string,
    [t],
  );
  const translateAny = useCallback(
    (key: string, options?: Record<string, unknown>) =>
      t(key as never, options as never) as unknown as string,
    [t],
  );
  const [request, setRequest] = useState<UsageHistoryRequest>({ kind: 'preset', preset: '24h' });
  const [windowId, setWindowId] = useState<string | null>(null);
  const [isMutating, setIsMutating] = useState(false);
  const [isClearDialogOpen, setIsClearDialogOpen] = useState(false);
  const [isCustomDialogOpen, setIsCustomDialogOpen] = useState(false);
  const [appliedCustomDates, setAppliedCustomDates] = useState<{ start: Dayjs; end: Dayjs } | null>(null);
  const initialCustomDates = useMemo(() => defaultCustomHistoryDates(now()), [now]);
  const [draftStart, setDraftStart] = useState<Dayjs | null>(() => dayjs(initialCustomDates.start));
  const [draftEnd, setDraftEnd] = useState<Dayjs | null>(() => dayjs(initialCustomDates.end));
  const [startPickerError, setStartPickerError] = useState<DateValidationError>(null);
  const [endPickerError, setEndPickerError] = useState<DateValidationError>(null);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [isFeedbackOpen, setIsFeedbackOpen] = useState(false);
  const customTriggerRef = useRef<HTMLButtonElement | null>(null);
  const isMutatingRef = useRef(false);
  const feedbackIdRef = useRef(0);
  const showFeedback = useCallback(
    (key: TrendsTranslationKey, severity: AlertColor) => {
      feedbackIdRef.current += 1;
      setFeedback({ id: feedbackIdRef.current, key, severity });
      setIsFeedbackOpen(true);
    },
    [setFeedback, setIsFeedbackOpen],
  );
  const beginMutation = useCallback(() => {
    if (isMutatingRef.current) return false;
    isMutatingRef.current = true;
    setIsMutating(true);
    return true;
  }, []);
  const finishMutation = useCallback(() => {
    isMutatingRef.current = false;
    setIsMutating(false);
  }, []);
  const onListenerError = useCallback(() => showFeedback('trends.events.error', 'error'), [showFeedback]);
  const { history, isLoading, loadFailed, requestReload, changeRange, clearSnapshot } = useHistoryQuery(
    request,
    { getUsageHistory, listenForUsageHistory },
    onListenerError,
  );
  const setCollectionEnabled = async (enabled: boolean) => {
    if (!beginMutation()) return;
    try {
      await onSetHistoryEnabled(enabled);
      showFeedback(
        enabled ? 'trends.collection.enableSuccess' : 'trends.collection.disableSuccess',
        'success',
      );
    } catch {
      showFeedback('trends.collection.error', 'error');
    } finally {
      finishMutation();
    }
  };

  const clearHistory = async () => {
    if (!beginMutation()) return;
    try {
      await onClearUsageHistory();
      clearSnapshot();
      setIsClearDialogOpen(false);
      showFeedback('trends.clear.success', 'success');
    } catch {
      showFeedback('trends.clear.error', 'error');
    } finally {
      finishMutation();
    }
  };

  const isEmpty = !isLoading && !loadFailed && (history?.series.length ?? 0) === 0;
  const rangeChoice: RangeChoice = request.kind === 'preset' ? request.preset : 'custom';
  const draftValidation = validateCustomHistoryDates(
    draftStart?.toDate() ?? null,
    draftEnd?.toDate() ?? null,
    now(),
  );
  const derivedPickerIssue = pickerIssue(startPickerError) ?? pickerIssue(endPickerError);
  const customIssue = derivedPickerIssue ?? draftValidation.issue;
  const customHelperText = customIssue ? translate(customIssueKey(customIssue)) : ' ';
  const isPartialCoverage = history
    ? hasPartialHistoryCoverage({
        truncatedByRetention: history.truncatedByRetention,
        appliedStartAt: history.appliedStartAt,
        availableStartAt: history.availableStartAt,
      })
    : false;

  const selectPreset = (preset: UsageHistoryPreset) => {
    if (rangeChoice === preset) return;
    changeRange();
    setRequest({ kind: 'preset', preset });
  };

  const openCustomRange = () => {
    const fallback = defaultCustomHistoryDates(now());
    setDraftStart(appliedCustomDates?.start ?? dayjs(fallback.start));
    setDraftEnd(appliedCustomDates?.end ?? dayjs(fallback.end));
    setStartPickerError(null);
    setEndPickerError(null);
    setIsCustomDialogOpen(true);
  };

  const selectedIndex = Math.max(0, history?.series.findIndex((series) => series.windowId === windowId) ?? 0);
  const selectedSeries = history?.series[selectedIndex];

  const applyCustomRange = () => {
    const applyInstant = now();
    const start = draftStart?.toDate() ?? null;
    const end = draftEnd?.toDate() ?? null;
    const validation = validateCustomHistoryDates(start, end, applyInstant);
    if (!start || !end || !validation.valid || startPickerError || endPickerError) return;
    setAppliedCustomDates({ start: draftStart!, end: draftEnd! });
    changeRange();
    setRequest(buildCustomHistoryRequest(start, end, applyInstant));
    setIsCustomDialogOpen(false);
  };

  return (
    <Stack component="section" spacing={2.25} aria-labelledby="trends-page-title">
      <Stack
        direction={{ xs: 'column', md: 'row' }}
        spacing={1.5}
        sx={{ justifyContent: 'space-between', alignItems: { xs: 'stretch', md: 'center' } }}
      >
        <Box>
          <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
            <QueryStatsRoundedIcon color="primary" />
            <Typography id="trends-page-title" variant="h5" sx={{ fontWeight: 800 }}>
              {translate('trends.title')}
            </Typography>
          </Stack>
          <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
            {translate('trends.subtitle')}
          </Typography>
        </Box>
        <ToggleButtonGroup
          exclusive
          size="small"
          value={rangeChoice}
          onChange={(_, nextChoice: RangeChoice | null) => {
            if (!nextChoice) {
              if (rangeChoice === 'custom') openCustomRange();
              return;
            }
            if (nextChoice === 'custom') openCustomRange();
            else selectPreset(nextChoice);
          }}
          aria-label={translate('trends.range.aria')}
          sx={{
            alignSelf: { xs: 'stretch', md: 'auto' },
            display: 'grid',
            gridTemplateColumns: { xs: 'repeat(2, minmax(0, 1fr))', md: 'repeat(5, auto)' },
            '& .MuiToggleButtonGroup-grouped': {
              minWidth: 0,
              m: 0,
              border: 1,
              borderColor: 'divider',
              borderRadius: 0,
              '&:nth-of-type(1)': {
                borderTopLeftRadius: 10,
                borderBottomLeftRadius: { md: 10 },
              },
              '&:nth-of-type(2)': { borderTopRightRadius: { xs: 10, md: 0 } },
              '&:nth-of-type(5)': {
                gridColumn: { xs: '1 / -1', md: 'auto' },
                borderBottomLeftRadius: { xs: 10, md: 0 },
                borderBottomRightRadius: 10,
                borderTopRightRadius: { md: 10 },
              },
            },
          }}
        >
          <ToggleButton value="24h">{translate('trends.range.hours24')}</ToggleButton>
          <ToggleButton value="7d">{translate('trends.range.days7')}</ToggleButton>
          <ToggleButton value="30d">{translate('trends.range.days30')}</ToggleButton>
          <ToggleButton value="all">{translate('trends.range.all')}</ToggleButton>
          <ToggleButton value="custom" ref={customTriggerRef}>
            {rangeChoice === 'custom'
              ? translate('trends.range.customApplied')
              : translate('trends.range.custom')}
          </ToggleButton>
        </ToggleButtonGroup>
      </Stack>

      <Card variant="outlined">
        <CardContent sx={{ p: 2, '&:last-child': { pb: 2 } }}>
          <Stack
            direction={{ xs: 'column', sm: 'row' }}
            spacing={2}
            sx={{ justifyContent: 'space-between', alignItems: { xs: 'stretch', sm: 'center' } }}
          >
            <Stack direction="row" spacing={1.25} sx={{ alignItems: 'flex-start' }}>
              <StorageRoundedIcon color="primary" sx={{ mt: 0.2 }} />
              <Box>
                <Typography variant="subtitle2" sx={{ fontWeight: 800 }}>
                  {translate('trends.localOnly.title')}
                </Typography>
                <Typography variant="body2" color="text.secondary" sx={{ mt: 0.25 }}>
                  {translate('trends.localOnly.description')}
                </Typography>
              </Box>
            </Stack>
            <Stack direction="row" spacing={1} sx={{ alignItems: 'center', flexShrink: 0 }}>
              <FormControlLabel
                sx={{ m: 0 }}
                control={
                  <Switch
                    checked={historyEnabled}
                    disabled={isMutating}
                    onChange={(event) => void setCollectionEnabled(event.target.checked)}
                  />
                }
                label={translate('trends.collection.label')}
              />
              <Button
                color="error"
                variant="outlined"
                size="small"
                startIcon={<DeleteOutlineRoundedIcon />}
                disabled={isMutating}
                onClick={() => setIsClearDialogOpen(true)}
              >
                {translate('trends.clear.button')}
              </Button>
            </Stack>
          </Stack>
          <Alert severity={historyEnabled ? 'info' : 'warning'} sx={{ mt: 1.5 }}>
            {translate(historyEnabled ? 'trends.collection.enabled' : 'trends.collection.disabled')}
          </Alert>
        </CardContent>
      </Card>

      {isLoading && <LinearProgress aria-label={translate('trends.loading')} />}

      {history?.storageStatus === 'recovered' && (
        <Alert severity="warning">{translate('trends.storage.recovered')}</Alert>
      )}
      {history?.storageStatus === 'unavailable' && (
        <Alert severity="error">{translate('trends.storage.unavailable')}</Alert>
      )}
      {isPartialCoverage && (
        <Alert severity="info">
          {history?.availableStartAt
            ? translate('trends.partialCoverageFrom', {
                date: formatDateTime(history.availableStartAt, language),
              })
            : translate('trends.partialCoverage')}
        </Alert>
      )}

      {loadFailed && (
        <Alert
          severity="error"
          action={
            <Button color="inherit" size="small" onClick={requestReload}>
              {translate('trends.retry')}
            </Button>
          }
        >
          {translate('trends.loadError')}
        </Alert>
      )}

      {isLoading && !history && (
        <Stack spacing={1} sx={{ minHeight: 180, alignItems: 'center', justifyContent: 'center' }}>
          <CircularProgress size={28} />
          <Typography variant="body2" color="text.secondary">
            {translate('trends.loading')}
          </Typography>
        </Stack>
      )}

      {isEmpty && (
        <Box sx={{ py: 5, textAlign: 'center' }}>
          <Typography variant="h6" sx={{ fontWeight: 800 }}>
            {translate('trends.empty.title')}
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ mt: 0.75 }}>
            {translate(
              historyEnabled ? 'trends.empty.enabledDescription' : 'trends.empty.disabledDescription',
            )}
          </Typography>
        </Box>
      )}

      {!loadFailed && history && history.series.length > 0 && (
        <>
          <Typography variant="caption" color="text.secondary">
            {translate('trends.summary', {
              count: history.sampleCount,
              date: history.latestSampleAt
                ? formatDateTime(history.latestSampleAt, language)
                : translateAny('common.unavailable'),
            })}
          </Typography>
          <Stack spacing={2}>
            {history.series.length > 1 && (
              <ToggleButtonGroup
                exclusive
                size="small"
                value={selectedIndex}
                aria-label={translateAny('feature.quotaWindow')}
                onChange={(_, index: number | null) => {
                  if (index !== null) setWindowId(history.series[index].windowId);
                }}
              >
                {history.series.map((series, index) => (
                  <ToggleButton key={series.windowId} value={index}>
                    {fallbackWindowLabel(series.fallbackLabel, series.windowSeconds, language, translateAny)}
                  </ToggleButton>
                ))}
              </ToggleButtonGroup>
            )}
            {selectedSeries && (
              <SeriesCard
                accountId={accountId}
                key={`${history.accountId}-${selectedSeries.windowId}-${JSON.stringify(history.request)}`}
                request={history.request}
                series={selectedSeries}
                language={language}
                translate={translate}
                translateAny={translateAny}
              />
            )}
          </Stack>
        </>
      )}

      <Dialog
        open={isCustomDialogOpen}
        onClose={() => setIsCustomDialogOpen(false)}
        aria-labelledby="custom-history-dialog-title"
        aria-describedby="custom-history-dialog-description"
        maxWidth="sm"
        fullWidth
        slotProps={{
          transition: {
            onExited: () => customTriggerRef.current?.focus(),
          },
        }}
      >
        <DialogTitle id="custom-history-dialog-title">{translate('trends.custom.dialogTitle')}</DialogTitle>
        <DialogContent>
          <Typography
            id="custom-history-dialog-description"
            variant="body2"
            color="text.secondary"
            sx={{ mb: 2 }}
          >
            {translate('trends.custom.dialogDescription')}
          </Typography>
          <LocalizationProvider
            dateAdapter={AdapterDayjs}
            adapterLocale={language === 'zh-CN' ? 'zh-cn' : 'en'}
            localeText={
              (language === 'zh-CN' ? zhCN : enUS).components.MuiLocalizationProvider.defaultProps.localeText
            }
          >
            <Box
              sx={{
                display: 'grid',
                gridTemplateColumns: 'repeat(auto-fit, minmax(min(220px, 100%), 1fr))',
                gap: 2,
                pt: 0.5,
              }}
            >
              <DesktopDatePicker
                label={translate('trends.custom.startLabel')}
                value={draftStart}
                onChange={(value, context: PickerChangeHandlerContext<DateValidationError>) => {
                  setDraftStart(value);
                  setStartPickerError(context.validationError);
                }}
                onError={setStartPickerError}
                maxDate={dayjs(now())}
                timezone="system"
                autoFocus
                slotProps={{
                  textField: {
                    id: 'custom-history-start-date',
                    fullWidth: true,
                    size: 'small',
                    error: customIssue !== null,
                    helperText: customHelperText,
                  },
                }}
              />
              <DesktopDatePicker
                label={translate('trends.custom.endLabel')}
                value={draftEnd}
                onChange={(value, context: PickerChangeHandlerContext<DateValidationError>) => {
                  setDraftEnd(value);
                  setEndPickerError(context.validationError);
                }}
                onError={setEndPickerError}
                maxDate={dayjs(now())}
                timezone="system"
                slotProps={{
                  textField: {
                    id: 'custom-history-end-date',
                    fullWidth: true,
                    size: 'small',
                    error: customIssue !== null,
                    helperText: ' ',
                  },
                }}
              />
            </Box>
          </LocalizationProvider>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setIsCustomDialogOpen(false)}>{translate('trends.custom.cancel')}</Button>
          <Button variant="contained" disabled={customIssue !== null} onClick={applyCustomRange}>
            {translate('trends.custom.apply')}
          </Button>
        </DialogActions>
      </Dialog>

      <Dialog
        open={isClearDialogOpen}
        onClose={() => {
          if (!isMutating) setIsClearDialogOpen(false);
        }}
        aria-labelledby="clear-history-dialog-title"
        maxWidth="xs"
        fullWidth
      >
        <DialogTitle id="clear-history-dialog-title">{translate('trends.clear.dialogTitle')}</DialogTitle>
        <DialogContent>
          <Typography variant="body2" color="text.secondary">
            {translate('trends.clear.dialogBody')}
          </Typography>
        </DialogContent>
        <DialogActions>
          <Button disabled={isMutating} onClick={() => setIsClearDialogOpen(false)}>
            {translate('trends.clear.cancel')}
          </Button>
          <Button color="error" variant="contained" disabled={isMutating} onClick={() => void clearHistory()}>
            {translate('trends.clear.confirm')}
          </Button>
        </DialogActions>
      </Dialog>

      <Snackbar
        key={feedback?.id ?? 0}
        open={feedback !== null && isFeedbackOpen}
        autoHideDuration={3_500}
        onClose={(_event, reason) => {
          if (reason !== 'clickaway') setIsFeedbackOpen(false);
        }}
        anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
      >
        <Alert
          closeText={translateAny('common.close')}
          severity={feedback?.severity ?? 'info'}
          variant="filled"
          onClose={() => setIsFeedbackOpen(false)}
          sx={{
            color: theme.palette.getContrastText(theme.palette[feedback?.severity ?? 'info'].main),
          }}
        >
          {feedback ? translate(feedback.key) : ''}
        </Alert>
      </Snackbar>
    </Stack>
  );
}
