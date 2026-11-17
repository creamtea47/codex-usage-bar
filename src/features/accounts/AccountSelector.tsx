import AccountCircleOutlinedIcon from '@mui/icons-material/AccountCircleOutlined';
import {
  Alert,
  Box,
  IconButton,
  ListItemText,
  Menu,
  MenuItem,
  Snackbar,
  TextField,
  Tooltip,
  Typography,
} from '@mui/material';
import CheckRoundedIcon from '@mui/icons-material/CheckRounded';
import ComputerOutlinedIcon from '@mui/icons-material/ComputerOutlined';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { accountActions } from './accountActions';
import { useAccounts } from './accountStore';
import { accountError } from './accountError';
import type { Account } from '@/contracts/accounts';
import { planLabel } from '@/shared/format';

import { accountTitle, accountNote } from './accountPresentation';

/** sidebar 展示完整身份；主卡保持原有布局、备注与脱敏数据。 */
export function AccountSelector({ sidebar = false }: { sidebar?: boolean }) {
  const { t } = useTranslation();
  const data = useAccounts();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  if (!data.accounts.length) return null;
  return (
    <Box data-no-drag sx={{ py: sidebar ? 0 : 1 }}>
      <TextField
        select
        size="small"
        fullWidth
        label={t('feature.selected')}
        value={data.selectedId ?? ''}
        disabled={busy}
        slotProps={{
          select: {
            renderValue: (value) => {
              const account = data.accounts.find((a) => a.id === value);
              if (!account) return '';
              return (
                <Tooltip title={sidebar ? accountTitle(account) : account.label}>
                  <Typography variant="body2" noWrap>
                    {sidebar ? accountTitle(account) : account.label}
                  </Typography>
                </Tooltip>
              );
            },
            MenuProps: { slotProps: { paper: { sx: { maxWidth: 420 } } } },
          },
        }}
        onChange={(event) => {
          setBusy(true);
          setError(null);
          void accountActions
            .select(event.target.value)
            .catch((e) => setError(accountError(e)))
            .finally(() => setBusy(false));
        }}
      >
        {data.accounts.map((a) => (
          <MenuItem key={a.id} value={a.id} sx={{ gap: 1, minWidth: 0 }}>
            <ListItemText
              primary={sidebar ? accountTitle(a) : a.label}
              secondary={
                sidebar
                  ? [accountNote(a), planLabel(a.details?.planType ?? a.dashboard.planLabel)]
                      .filter(Boolean)
                      .join(' · ')
                  : undefined
              }
              slotProps={{ primary: { noWrap: true }, secondary: { noWrap: true } }}
            />
            {sidebar && data.codexLogin?.matchedAccountId === a.id && (
              <Tooltip title={t('feature.accountUi.codexFile')}>
                <ComputerOutlinedIcon color="primary" fontSize="small" />
              </Tooltip>
            )}
          </MenuItem>
        ))}
      </TextField>
      {error && (
        <Alert severity="error" sx={{ mt: 1 }}>
          {error}
        </Alert>
      )}
    </Box>
  );
}

/** 标题栏只保留图标；菜单和错误提示不参与主卡高度计算。 */
export function AccountSwitcher() {
  const { t } = useTranslation();
  const data = useAccounts();
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const selected = data.accounts.find((a) => a.id === data.selectedId);
  const identity = (a: Account) => a.dashboard.accountEmailMasked ?? a.label;
  const title = selected
    ? t('feature.compact.currentAccount', { account: identity(selected) })
    : t('feature.compact.noAccounts');
  return (
    <>
      <Tooltip title={title}>
        <span data-no-drag>
          <IconButton
            size="small"
            color="inherit"
            aria-label={t('feature.compact.switchAccount')}
            aria-haspopup="menu"
            aria-expanded={!!anchor}
            aria-controls={anchor ? 'account-switch-menu' : undefined}
            disabled={busy || data.accounts.length < 2}
            onClick={(e) => setAnchor(e.currentTarget)}
          >
            <AccountCircleOutlinedIcon fontSize="small" />
          </IconButton>
        </span>
      </Tooltip>
      <Menu
        id="account-switch-menu"
        data-no-drag
        anchorEl={anchor}
        open={!!anchor}
        onClose={() => !busy && setAnchor(null)}
        slotProps={{
          list: { 'aria-label': t('feature.compact.switchAccount') },
          paper: { sx: { maxWidth: 320 } },
        }}
      >
        {data.accounts.map((account) => (
          <MenuItem
            key={account.id}
            role="menuitemradio"
            aria-checked={account.id === data.selectedId}
            selected={account.id === data.selectedId}
            disabled={busy}
            sx={{ gap: 1 }}
            onClick={() => {
              if (account.id === data.selectedId) {
                setAnchor(null);
                return;
              }
              setBusy(true);
              setError(null);
              void accountActions
                .select(account.id)
                .then(() => setAnchor(null))
                .catch((e) => {
                  setError(accountError(e));
                  setAnchor(null);
                })
                .finally(() => setBusy(false));
            }}
          >
            <ListItemText
              primary={identity(account)}
              secondary={[
                account.label !== identity(account) ? account.label : null,
                planLabel(account.dashboard.planLabel),
              ]
                .filter(Boolean)
                .join(' · ')}
              slotProps={{ primary: { noWrap: true }, secondary: { noWrap: true } }}
            />
            {account.id === data.selectedId && <CheckRoundedIcon fontSize="small" />}
          </MenuItem>
        ))}
      </Menu>
      <Snackbar open={!!error} autoHideDuration={5000} onClose={() => setError(null)}>
        <Alert severity="error" onClose={() => setError(null)}>
          {error}
        </Alert>
      </Snackbar>
    </>
  );
}
