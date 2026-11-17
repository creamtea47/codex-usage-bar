import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { getCurrentWindow } from '@/platform/desktop';

const loadWindow =
  getCurrentWindow().label === 'settings' ? import('./app/SettingsWindow') : import('./app/WidgetWindow');

void loadWindow.then(({ default: Window }) => {
  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <Window />
    </StrictMode>,
  );
});
