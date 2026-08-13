import { Alert, Button, LinearProgress } from '@mui/material';
import { Component, Fragment, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { usageBridge } from './bridge';

interface LocalErrorBoundaryProps {
  active: boolean;
  children: ReactNode;
  fallback: (retry: () => void) => ReactNode;
  recovering: ReactNode;
  recoveryRevision: number;
  onTerminalError: () => void;
}

interface LocalErrorBoundaryState {
  autoRetriedRevision: number | null;
  childEpoch: number;
  failed: boolean;
  recoveryRevision: number;
}

/**
 * Keeps retry bookkeeping outside the Trends subtree. A confirmed operation grants
 * at most one automatic remount; a persistent error then remains terminal until a
 * user retry or a newer confirmed operation arrives.
 */
class LocalErrorBoundary extends Component<LocalErrorBoundaryProps, LocalErrorBoundaryState> {
  state: LocalErrorBoundaryState = {
    autoRetriedRevision: null,
    childEpoch: 0,
    failed: false,
    recoveryRevision: this.props.recoveryRevision,
  };

  private lastReportedRevision: number | null = null;
  private recoveryTask: ReturnType<typeof setTimeout> | null = null;

  static getDerivedStateFromProps(
    props: LocalErrorBoundaryProps,
    state: LocalErrorBoundaryState,
  ): Partial<LocalErrorBoundaryState> | null {
    if (props.recoveryRevision <= state.recoveryRevision) return null;
    if (state.failed) {
      return {
        autoRetriedRevision: props.recoveryRevision,
        childEpoch: state.childEpoch + 1,
        failed: false,
        recoveryRevision: props.recoveryRevision,
      };
    }
    return {
      autoRetriedRevision: null,
      recoveryRevision: props.recoveryRevision,
    };
  }

  static getDerivedStateFromError(): Partial<LocalErrorBoundaryState> {
    return { failed: true };
  }

  componentDidCatch() {
    const { autoRetriedRevision, recoveryRevision } = this.state;
    if (autoRetriedRevision !== recoveryRevision) {
      this.recoveryTask = setTimeout(() => {
        this.recoveryTask = null;
        this.setState((current) => {
          if (!current.failed || current.autoRetriedRevision === current.recoveryRevision) {
            return null;
          }
          return {
            autoRetriedRevision: current.recoveryRevision,
            childEpoch: current.childEpoch + 1,
            failed: false,
          };
        });
      }, 0);
      return;
    }

    if (this.lastReportedRevision !== recoveryRevision) {
      this.lastReportedRevision = recoveryRevision;
      this.props.onTerminalError();
    }
  }

  componentWillUnmount() {
    if (this.recoveryTask !== null) clearTimeout(this.recoveryTask);
  }

  private retry = () => {
    this.setState((current) => {
      if (!current.failed) return null;
      return {
        autoRetriedRevision: current.recoveryRevision,
        childEpoch: current.childEpoch + 1,
        failed: false,
      };
    });
  };

  render() {
    if (!this.props.active) return null;
    if (this.state.failed) {
      return this.state.autoRetriedRevision === this.state.recoveryRevision
        ? this.props.fallback(this.retry)
        : this.props.recovering;
    }
    return <Fragment key={this.state.childEpoch}>{this.props.children}</Fragment>;
  }
}

interface TrendsErrorBoundaryProps {
  active?: boolean;
  children: ReactNode;
  recoveryRevision: number;
}

/** Settings-window boundary with a bounded automatic recovery before its manual fallback. */
export default function TrendsErrorBoundary({
  active = true,
  children,
  recoveryRevision,
}: TrendsErrorBoundaryProps) {
  const { t } = useTranslation();

  return (
    <LocalErrorBoundary
      active={active}
      recoveryRevision={recoveryRevision}
      onTerminalError={() => {
        void usageBridge.reportSettingsUiFault('trends-render-failed').catch(() => undefined);
      }}
      recovering={<LinearProgress aria-label={t('trends.recovering')} />}
      fallback={(retry) => (
        <Alert
          severity="error"
          action={
            <Button color="inherit" size="small" onClick={retry}>
              {t('trends.retryDisplay')}
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
