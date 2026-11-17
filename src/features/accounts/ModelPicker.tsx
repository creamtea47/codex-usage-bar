import { Alert, Autocomplete, Button, Stack, TextField, Typography } from '@mui/material';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { accountActions } from './accountActions';
import { accountError } from './accountError';
import type { Account, ModelOption } from '@/contracts/accounts';

export function ModelPicker({ account, disabled = false }: { account: Account; disabled?: boolean }) {
  const { t } = useTranslation();
  const [models, setModels] = useState<ModelOption[]>([]);
  const [model, setModel] = useState(account.model ?? '');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  async function load() {
    setBusy(true);
    setError(null);
    try {
      setModels(await accountActions.models(account.id));
    } catch {
      setError(t('feature.noModels'));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Stack spacing={1.25}>
      <Typography variant="subtitle2">{t('feature.model')}</Typography>
      <Autocomplete
        freeSolo
        options={models.map((v) => v.id)}
        inputValue={model}
        value={model || null}
        disabled={busy || disabled}
        onInputChange={(_, value) => {
          setModel(value);
          setSaved(false);
        }}
        renderInput={(params) => (
          <TextField {...params} label={t('feature.modelId')} placeholder={t('feature.auto')} size="small" />
        )}
      />
      <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap', gap: 1 }}>
        <Button disabled={busy || disabled} onClick={() => void load()}>
          {t('feature.refreshModels')}
        </Button>
        <Button
          disabled={busy || disabled}
          onClick={() => {
            setModel('');
            setSaved(false);
          }}
        >
          {t('feature.auto')}
        </Button>
        <Button
          variant="outlined"
          disabled={busy || disabled}
          onClick={() => {
            setBusy(true);
            setError(null);
            void accountActions
              .update({ ...account, model: model.trim() || null })
              .then(() => setSaved(true))
              .catch((e) => setError(accountError(e)))
              .finally(() => setBusy(false));
          }}
        >
          {t('feature.save')}
        </Button>
      </Stack>
      <Typography variant="caption" color="text.secondary">
        {t('feature.modelHelp')}
      </Typography>
      {error && <Alert severity="warning">{error}</Alert>}
      {saved && <Alert severity="success">{t('feature.saved')}</Alert>}
    </Stack>
  );
}

/** 身份、额度和订阅优先呈现；认证细节折叠，底部只保留已有操作。 */
