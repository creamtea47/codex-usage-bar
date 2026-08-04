import assert from 'node:assert/strict';
import test from 'node:test';

import { buildUpdaterManifest, updaterAssets } from './generate-updater-manifest.mjs';

test('builds all four native updater targets with signatures', () => {
  const signatures = Object.fromEntries(updaterAssets.map(({ target }) => [target, `signature-${target}`]));
  const manifest = buildUpdaterManifest({
    version: '0.2.6',
    tag: 'v0.2.6',
    publishedAt: '2030-01-04T12:00:00.000Z',
    signatures,
  });

  assert.deepEqual(Object.keys(manifest.platforms), updaterAssets.map(({ target }) => target));
  assert.match(manifest.platforms['windows-x86_64'].url, /CodexUsageBar-x64-setup\.exe$/);
  assert.match(manifest.platforms['darwin-aarch64'].url, /CodexUsageBar-macos-arm64\.app\.tar\.gz$/);
  assert.equal(manifest.platforms['darwin-x86_64'].signature, 'signature-darwin-x86_64');
});

test('rejects tags and signature sets that cannot produce a trustworthy manifest', () => {
  assert.throws(
    () =>
      buildUpdaterManifest({
        version: '0.2.6',
        tag: 'v0.2.5',
        publishedAt: '2030-01-04T12:00:00.000Z',
        signatures: {},
      }),
    /does not match/,
  );

  assert.throws(
    () =>
      buildUpdaterManifest({
        version: '0.2.6',
        tag: 'v0.2.6',
        publishedAt: '2030-01-04T12:00:00.000Z',
        signatures: { 'windows-x86_64': 'only-one-signature' },
      }),
    /Missing updater signature/,
  );
});
