import { Alert, Button } from '@mui/material';
import { Component, Fragment, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { usageBridge } from './bridge';

interface LocalErrorBoundaryProps {
  children: ReactNode;
  fallback: (retry: () => void) => ReactNode;
  onError: () => void;
}

interface LocalErrorBoundaryState {
  failed: boolean;
  retryVersion: number;
}

/** 将趋势渲染故障限制在内容区；上报时只传固定故障码，不传异常正文或组件栈。 */
class LocalErrorBoundary extends Component<LocalErrorBoundaryProps, LocalErrorBoundaryState> {
  state: LocalErrorBoundaryState = { failed: false, retryVersion: 0 };

  static getDerivedStateFromError(): Partial<LocalErrorBoundaryState> {
    return { failed: true };
  }

  componentDidCatch() {
    this.props.onError();
  }

  private retry = () => {
    this.setState((current) => ({ failed: false, retryVersion: current.retryVersion + 1 }));
  };

  render() {
    if (this.state.failed) return this.props.fallback(this.retry);
    return <Fragment key={this.state.retryVersion}>{this.props.children}</Fragment>;
  }
}

/** 设置窗口趋势区域的脱敏错误边界，失败时仍保留侧栏与其他设置页面。 */
export default function TrendsErrorBoundary({ children }: { children: ReactNode }) {
  const { t } = useTranslation();

  return (
    <LocalErrorBoundary
      onError={() => {
        void usageBridge.reportSettingsUiFault('trends-render-failed').catch(() => undefined);
      }}
      fallback={(retry) => (
        <Alert
          severity="error"
          action={
            <Button color="inherit" size="small" onClick={retry}>
              {t('trends.retry')}
            </Button>
          }
        >
          {t('trends.renderError')}
        </Alert>
      )}
    >
      {children}
    </LocalErrorBoundary>
  );
}
