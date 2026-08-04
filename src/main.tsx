import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { getCurrentWindow } from '@tauri-apps/api/window';

import App from './App';
import SettingsWindow from './SettingsWindow';

// 两个 Tauri 窗口复用同一前端入口，通过不可伪造的窗口标签分派页面。
const isSettingsWindow = getCurrentWindow().label === 'settings';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {isSettingsWindow ? <SettingsWindow /> : <App />}
  </StrictMode>,
);
