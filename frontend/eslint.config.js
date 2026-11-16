import js from '@eslint/js'
import globals from 'globals'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import tseslint from 'typescript-eslint'
import { defineConfig, globalIgnores } from 'eslint/config'

export default defineConfig([
  globalIgnores(['dist']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [
      js.configs.recommended,
      ...tseslint.configs.recommended,
      reactHooks.configs.flat.recommended,
      reactRefresh.configs.vite,
    ],
    languageOptions: {
      globals: globals.browser,
      parserOptions: { ecmaFeatures: { jsx: true } },
    },
    rules: {
      '@typescript-eslint/no-explicit-any': 'error',
    },
  },
  {
    // `BackupPlanDetail` exports the three readers `BackupPanel` reads a plan body with,
    // next to the component that uses them; the header comment in that file says why they
    // live there and not in a second file. Named one by one instead of an in-file
    // `eslint-disable` so the rule keeps protecting the rest of the module. Flat config
    // replaces rule options rather than merging them, so `allowConstantExport: true` —
    // which `reactRefresh.configs.vite` sets for every other file — has to be restated
    // here or it is silently lost.
    files: ['src/components/BackupPlanDetail.tsx'],
    rules: {
      'react-refresh/only-export-components': [
        'error',
        { allowConstantExport: true, allowExportNames: ['countList', 'isCounts', 'isPlanSide'] },
      ],
    },
  },
  {
    files: ['**/*.{js,jsx}'],
    extends: [
      js.configs.recommended,
      reactHooks.configs.flat.recommended,
      reactRefresh.configs.vite,
    ],
    languageOptions: {
      globals: globals.browser,
      parserOptions: { ecmaFeatures: { jsx: true } },
    },
  },
])

