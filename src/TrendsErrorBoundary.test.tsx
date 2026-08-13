import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Component, type ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import TrendsErrorBoundary from './TrendsErrorBoundary';

const bridgeMocks = vi.hoisted(() => ({ reportSettingsUiFault: vi.fn() }));

vi.mock('./bridge', () => ({ usageBridge: bridgeMocks }));
vi.mock('react-i18next', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-i18next')>();
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string) =>
        ({
          'trends.renderError': 'The Trends page still cannot be displayed.',
          'trends.recovering': 'Recovering the Trends page.',
          'trends.retry': 'Retry',
          'trends.retryDisplay': 'Retry display',
        })[key] ?? key,
    }),
  };
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.clearAllMocks();
});

class ControlledRenderFailure extends Component<{
  children: ReactNode;
  message: string;
  shouldThrow: boolean;
}> {
  render() {
    if (this.props.shouldThrow) throw new Error(this.props.message);
    return this.props.children;
  }
}

describe('TrendsErrorBoundary', () => {
  it('automatically remounts once for a transient render failure without showing the terminal fallback', async () => {
    vi.useFakeTimers();
    const onCaughtError = vi.fn();
    const view = render(
      <TrendsErrorBoundary recoveryRevision={1}>
        <ControlledRenderFailure message="private transient render details" shouldThrow>
          <div>Recovered trends</div>
        </ControlledRenderFailure>
      </TrendsErrorBoundary>,
      { onCaughtError },
    );

    expect(screen.getByRole('progressbar', { name: 'Recovering the Trends page.' })).toBeTruthy();
    expect(screen.queryByText('The Trends page still cannot be displayed.')).toBeNull();
    view.rerender(
      <TrendsErrorBoundary recoveryRevision={1}>
        <ControlledRenderFailure
          message="private transient render details"
          shouldThrow={false}
        >
          <div>Recovered trends</div>
        </ControlledRenderFailure>
      </TrendsErrorBoundary>,
    );
    act(() => vi.runOnlyPendingTimers());

    expect(screen.getByText('Recovered trends')).toBeTruthy();
    expect(screen.queryByText('The Trends page still cannot be displayed.')).toBeNull();
    expect(bridgeMocks.reportSettingsUiFault).not.toHaveBeenCalled();
    expect(onCaughtError).toHaveBeenCalled();
  });

  it('bounds automatic recovery to one remount per revision and reports one terminal fault', async () => {
    bridgeMocks.reportSettingsUiFault.mockRejectedValue(new Error('reporting unavailable'));
    const onCaughtError = vi.fn();
    const view = render(
      <aside aria-label="settings sidebar">
        Sidebar remains
        <TrendsErrorBoundary recoveryRevision={4}>
          <ControlledRenderFailure message="private render details" shouldThrow>
            <div>Recovered trends</div>
          </ControlledRenderFailure>
        </TrendsErrorBoundary>
      </aside>,
      { onCaughtError },
    );

    expect(await screen.findByText('The Trends page still cannot be displayed.')).toBeTruthy();
    expect(screen.getByText('Sidebar remains')).toBeTruthy();
    await waitFor(() =>
      expect(bridgeMocks.reportSettingsUiFault).toHaveBeenCalledWith('trends-render-failed'),
    );
    expect(screen.queryByText('private render details')).toBeNull();

    view.rerender(
      <aside aria-label="settings sidebar">
        Sidebar remains
        <TrendsErrorBoundary recoveryRevision={4}>
          <ControlledRenderFailure message="private render details" shouldThrow={false}>
            <div>Recovered trends</div>
          </ControlledRenderFailure>
        </TrendsErrorBoundary>
      </aside>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Retry display' }));
    expect(await screen.findByText('Recovered trends')).toBeTruthy();
    expect(bridgeMocks.reportSettingsUiFault).toHaveBeenCalledTimes(1);
    expect(onCaughtError).toHaveBeenCalled();
  });

  it('does not clear a terminal fallback for the same revision but remounts for a newer operation', async () => {
    const view = render(
      <TrendsErrorBoundary recoveryRevision={7}>
        <ControlledRenderFailure message="private persistent render details" shouldThrow>
          <div>Recovered for a new operation</div>
        </ControlledRenderFailure>
      </TrendsErrorBoundary>,
      { onCaughtError: vi.fn() },
    );
    expect(await screen.findByText('The Trends page still cannot be displayed.')).toBeTruthy();

    view.rerender(
      <TrendsErrorBoundary recoveryRevision={7}>
        <ControlledRenderFailure
          message="private persistent render details"
          shouldThrow={false}
        >
          <div>Recovered for a new operation</div>
        </ControlledRenderFailure>
      </TrendsErrorBoundary>,
    );
    expect(screen.getByText('The Trends page still cannot be displayed.')).toBeTruthy();

    view.rerender(
      <TrendsErrorBoundary recoveryRevision={8}>
        <ControlledRenderFailure
          message="private persistent render details"
          shouldThrow={false}
        >
          <div>Recovered for a new operation</div>
        </ControlledRenderFailure>
      </TrendsErrorBoundary>,
    );
    expect(await screen.findByText('Recovered for a new operation')).toBeTruthy();
    expect(bridgeMocks.reportSettingsUiFault).toHaveBeenCalledTimes(1);
  });

  it('preserves an exhausted recovery budget while navigating away and back', async () => {
    const view = render(
      <TrendsErrorBoundary active recoveryRevision={10}>
        <ControlledRenderFailure message="private persistent render details" shouldThrow>
          <div>Recovered after a confirmed operation</div>
        </ControlledRenderFailure>
      </TrendsErrorBoundary>,
      { onCaughtError: vi.fn() },
    );
    expect(await screen.findByText('The Trends page still cannot be displayed.')).toBeTruthy();
    await waitFor(() => expect(bridgeMocks.reportSettingsUiFault).toHaveBeenCalledTimes(1));

    view.rerender(
      <TrendsErrorBoundary active={false} recoveryRevision={10}>
        <ControlledRenderFailure message="private persistent render details" shouldThrow={false}>
          <div>Recovered after a confirmed operation</div>
        </ControlledRenderFailure>
      </TrendsErrorBoundary>,
    );
    expect(screen.queryByText('The Trends page still cannot be displayed.')).toBeNull();

    view.rerender(
      <TrendsErrorBoundary active recoveryRevision={10}>
        <ControlledRenderFailure message="private persistent render details" shouldThrow={false}>
          <div>Recovered after a confirmed operation</div>
        </ControlledRenderFailure>
      </TrendsErrorBoundary>,
    );
    expect(screen.getByText('The Trends page still cannot be displayed.')).toBeTruthy();
    expect(screen.queryByText('Recovered after a confirmed operation')).toBeNull();
    expect(bridgeMocks.reportSettingsUiFault).toHaveBeenCalledTimes(1);

    view.rerender(
      <TrendsErrorBoundary active recoveryRevision={11}>
        <ControlledRenderFailure message="private persistent render details" shouldThrow={false}>
          <div>Recovered after a confirmed operation</div>
        </ControlledRenderFailure>
      </TrendsErrorBoundary>,
    );
    expect(await screen.findByText('Recovered after a confirmed operation')).toBeTruthy();
    expect(bridgeMocks.reportSettingsUiFault).toHaveBeenCalledTimes(1);
  });
});
