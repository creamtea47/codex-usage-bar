import { createTheme, type PaletteMode } from '@mui/material/styles';

/**
 * CodexUsageBar 的视觉令牌集中在此处，避免组件各自维护一套颜色和圆角。
 * 主题只影响前端呈现，不触及 Rust 的认证、请求或本地设置边界。
 */
export function createUsageTheme(mode: PaletteMode) {
  const isDark = mode === 'dark';

  return createTheme({
    palette: {
      mode,
      primary: {
        main: isDark ? '#90caf9' : '#1565c0',
      },
      success: {
        main: isDark ? '#66bb6a' : '#2e7d32',
      },
      warning: {
        main: isDark ? '#ffb74d' : '#ed6c02',
      },
      error: {
        main: isDark ? '#f28b82' : '#d32f2f',
      },
      background: {
        default: 'transparent',
        paper: isDark ? '#1b1f24' : '#fbfcff',
      },
    },
    shape: {
      borderRadius: 16,
    },
    typography: {
      fontFamily: '"Segoe UI Variable", "Segoe UI", system-ui, sans-serif',
      button: {
        fontWeight: 700,
        textTransform: 'none',
      },
    },
    components: {
      MuiCssBaseline: {
        styleOverrides: {
          html: { width: '100%', height: '100%', backgroundColor: 'transparent' },
          body: { width: '100%', height: '100%', margin: 0, backgroundColor: 'transparent' },
          '#root': { width: '100%', height: '100%', backgroundColor: 'transparent' },
        },
      },
      MuiPaper: {
        styleOverrides: {
          root: {
            backgroundImage: 'none',
          },
        },
      },
      MuiButton: {
        defaultProps: {
          disableElevation: true,
        },
      },
    },
  });
}
