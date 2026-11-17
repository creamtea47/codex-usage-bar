# Repository Guidelines

## Project Structure & Module Organization

This Tauri 2 desktop widget uses React 19, TypeScript, Material UI, and Rust.

- `src/`: React pages/components, IPC bridges, shared types, localization, and colocated frontend tests.
- `src-tauri/src/`: credential handling, accounts, usage collection, history analytics, notifications, scheduling, and native window/tray integration.
- `src-tauri/`: Cargo manifests, Tauri configuration, capabilities, and bundle icons.
- `scripts/`: updater manifest generation, its Node tests, and a standalone Rust signature verifier.
- `docs/`: bilingual development guides, release notes, and screenshots; `.github/workflows/build.yml` validates and packages Windows/macOS builds.

## Build, Test, and Development Commands

Install Node.js 22.13+ or 24+, pnpm 10, and stable Rust. Native development requires Xcode Command Line Tools on macOS or MSVC and WebView2 on Windows.

- `pnpm install --frozen-lockfile`: install the pinned dependencies.
- `pnpm tauri dev`: launch the native app with frontend hot reload.
- `pnpm dev`: run the Vite frontend server on port 1420.
- `pnpm lint`: run ESLint with zero warnings allowed.
- `pnpm test`: run Vitest and updater-manifest Node tests.
- `pnpm test:watch`: watch tests.
- `pnpm build`: type-check TypeScript and build frontend assets.
- `cargo test --locked --manifest-path src-tauri/Cargo.toml`: run backend tests.

See `docs/guide.md` for packaging and signing.

## Coding Style & Naming Conventions

Follow existing TypeScript style: two-space indentation, single quotes, semicolons, PascalCase components, and camelCase functions/variables. Use snake_case Rust modules/functions and standard rustfmt formatting (`cargo fmt --manifest-path src-tauri/Cargo.toml`). ESLint enforces TypeScript and React Hooks rules. Keep English and Simplified Chinese UI strings synchronized in the localization resources.

## Testing Guidelines

Frontend tests use Vitest, jsdom, and React Testing Library; name them `*.test.ts` or `*.test.tsx` beside the implementation. Rust tests live in module-local `mod tests`, using `#[test]` or `#[tokio::test]`. Add regression coverage for changed behavior, mock native IPC where appropriate, and make clock/timezone assumptions explicit. No numeric coverage threshold is configured. Run lint, frontend tests/build, and backend tests before submitting code changes.

## Commit & Pull Request Guidelines

History uses prefixes such as `feat:`, `fix:`, `test:`, `docs:`, and `chore:`, with English or Chinese summaries. Keep commits focused. PRs should explain the change, link relevant issues, and report validation commands/results. Include OS/app version and reproduction steps for bugs, plus screenshots for UI changes.

## Security & Configuration

Keep credential access in Rust; never expose tokens or authentication files through frontend IPC or logs. Never commit `auth.json`, private account details, or signing secrets. Use sanitized diagnostics and synthetic test fixtures.
