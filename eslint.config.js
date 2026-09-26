import js from '@eslint/js'
import globals from 'globals'
import tseslint from 'typescript-eslint'
import reactHooks from 'eslint-plugin-react-hooks'
// The line's rule: a component names a colour from the dowel vocabulary and
// never writes one down, so the theme can swap it and the accent can move.
import dowel from 'dowel-ui/eslint'

export default tseslint.config(
  // `docs` is its own Astro project with its own toolchain, and most of what
  // lives there is generated. `target` is Cargo's build output: the workspace
  // root here, so it needs its own ignore rather than riding along with
  // `src-tauri`. `crates` is Rust.
  { ignores: ['dist', 'src-tauri', 'crates', 'docs', 'target'] },
  js.configs.recommended,
  tseslint.configs.recommended,
  // The `flat` variant; the top-level one is still in the legacy shape.
  reactHooks.configs.flat['recommended-latest'],
  ...dowel.configs.recommended,
  // The registry copies are dowel's, held byte for byte to the installed
  // package by `tools/check-registry.mjs`, and held to the scale by dowel's own
  // scale test with each exception argued there. The recommended config keeps
  // `no-raw-button` out of `ui/` but not this one, so dowel's own splash - a
  // 26px name that is off the scale on purpose - fails here, in a file that
  // may not be edited. dowel lints its registry the same way.
  {
    files: ['src/components/ui/**'],
    rules: { 'dowel/no-arbitrary-scale': 'off' },
  },
  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
  },
  // Build tooling runs under Node, not in the webview.
  {
    files: ['tools/**/*.mjs', '*.config.{js,ts}'],
    languageOptions: { globals: globals.node },
  },
)
