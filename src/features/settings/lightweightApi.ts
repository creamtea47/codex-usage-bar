import { invoke } from '@/platform/ipc';
export const lightweightApi = {
  measurement: () => invoke<string | null>('get_lightweight_measurement'),
  enter: () => invoke<void>('enter_lightweight'),
};
