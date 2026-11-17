export interface AppUpdateInfo {
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  checkedAt: string | null;
}

export type AppUpdateStage = 'downloading' | 'verifying' | 'installing';

/** 下载进度只用于呈现，不携带下载地址、签名或任何认证资料。 */
export interface AppUpdateProgress {
  stage: AppUpdateStage;
  downloadedBytes: number;
  totalBytes: number | null;
}
