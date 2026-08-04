import { readFile, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const repository = 'creamtea47/codex-usage-bar';

/**
 * Tauri 使用的 target 名称与面向用户的安装包名称并不完全一致。
 * macOS 自动更新必须下载 .app.tar.gz，而不是供手动安装的 DMG。
 */
export const updaterAssets = [
  { target: 'windows-x86_64', file: 'CodexUsageBar-x64-setup.exe' },
  { target: 'windows-aarch64', file: 'CodexUsageBar-arm64-setup.exe' },
  { target: 'darwin-x86_64', file: 'CodexUsageBar-macos-x64.app.tar.gz' },
  { target: 'darwin-aarch64', file: 'CodexUsageBar-macos-arm64.app.tar.gz' },
];

export function buildUpdaterManifest({ version, tag, publishedAt, signatures }) {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error(`Invalid release version: ${version}`);
  }
  if (tag !== `v${version}`) {
    throw new Error(`Tag ${tag} does not match version ${version}.`);
  }

  const platforms = Object.fromEntries(
    updaterAssets.map(({ target, file }) => {
      const signature = signatures[target]?.trim();
      if (!signature) throw new Error(`Missing updater signature for ${target}.`);
      return [
        target,
        {
          url: `https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(file)}`,
          signature,
        },
      ];
    }),
  );

  return {
    version,
    notes: `CodexUsageBar v${version} 已发布。详细变更请查看 GitHub Release。`,
    pub_date: publishedAt,
    platforms,
  };
}

async function assertProjectVersions(projectRoot, version) {
  const [packageJson, tauriConfig, cargoToml] = await Promise.all([
    readFile(path.join(projectRoot, 'package.json'), 'utf8'),
    readFile(path.join(projectRoot, 'src-tauri', 'tauri.conf.json'), 'utf8'),
    readFile(path.join(projectRoot, 'src-tauri', 'Cargo.toml'), 'utf8'),
  ]);
  const packageVersion = JSON.parse(packageJson).version;
  const tauriVersion = JSON.parse(tauriConfig).version;
  const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if ([packageVersion, tauriVersion, cargoVersion].some((candidate) => candidate !== version)) {
    throw new Error(
      `Version mismatch: package=${packageVersion}, tauri=${tauriVersion}, cargo=${cargoVersion}, tag=${version}.`,
    );
  }
}

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith('--') || !value) throw new Error('Usage: --assets-dir <dir> --tag <vX.Y.Z>');
    options[key.slice(2)] = value;
  }
  return options;
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  const assetsDir = options['assets-dir'];
  const tag = options.tag;
  if (!assetsDir || !tag) throw new Error('Usage: --assets-dir <dir> --tag <vX.Y.Z>');
  const version = tag.startsWith('v') ? tag.slice(1) : tag;
  await assertProjectVersions(process.cwd(), version);

  const signatures = Object.fromEntries(
    await Promise.all(
      updaterAssets.map(async ({ target, file }) => [
        target,
        await readFile(path.join(assetsDir, `${file}.sig`), 'utf8'),
      ]),
    ),
  );
  const manifest = buildUpdaterManifest({
    version,
    tag,
    publishedAt: new Date().toISOString(),
    signatures,
  });
  const outputPath = path.join(assetsDir, 'latest.json');
  await writeFile(outputPath, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');
  process.stdout.write(`Generated signed updater manifest: ${outputPath}\n`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`Unable to generate updater manifest: ${error.message}\n`);
    process.exitCode = 1;
  });
}
