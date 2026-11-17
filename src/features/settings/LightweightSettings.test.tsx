import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { LightweightSettings } from './LightweightSettings';
import i18n from '@/shared/i18n/i18n';
import { defaultSettings } from './defaultSettings';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
afterEach(cleanup);

it('shows the measured scope, saves delay changes and requests immediate release', async () => {
  await i18n.changeLanguage('en');
  vi.mocked(invoke).mockResolvedValue('280.0 MB → 110.0 MB (-170.0 MB · includes WebViews)');
  const update = vi.fn().mockResolvedValue(true);
  const { rerender } = render(
    <LightweightSettings settings={defaultSettings} disabled={false} update={update} />,
  );
  expect(await screen.findByText(/includes WebViews/)).toBeTruthy();
  const delay = () => screen.getByRole('spinbutton', { name: 'Delay (minutes)' }) as HTMLInputElement;
  expect(delay().value).toBe('10');
  expect(delay().disabled).toBe(true);
  fireEvent.click(screen.getByLabelText('Enter automatically after both windows are hidden'));
  expect(update).toHaveBeenCalledWith({ autoLightweight: true });

  rerender(
    <LightweightSettings
      settings={{ ...defaultSettings, autoLightweight: true }}
      disabled={false}
      update={update}
    />,
  );
  fireEvent.change(delay(), { target: { value: '3' } });
  fireEvent.blur(delay());
  expect(update).toHaveBeenCalledWith({ lightweightMinutes: 3 });
  fireEvent.click(screen.getByRole('button', { name: 'Enter lightweight mode now' }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('enter_lightweight'));
});
