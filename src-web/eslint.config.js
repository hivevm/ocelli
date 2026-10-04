// ESLint for the web interface (ADR-0008): the type check is svelte-check's; ESLint holds the
// size and complexity sensors the toolchain ADR names, on TypeScript and Svelte alike.
import ts from 'typescript-eslint';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';

export default ts.config(
  { ignores: ['node_modules/', '../dist/'] },
  ...svelte.configs['flat/base'],
  {
    files: ['**/*.ts'],
    languageOptions: { parser: ts.parser },
  },
  {
    files: ['**/*.svelte'],
    languageOptions: { parserOptions: { parser: ts.parser, extraFileExtensions: ['.svelte'] } },
  },
  {
    files: ['**/*.ts', '**/*.svelte'],
    languageOptions: { globals: globals.browser },
    rules: {
      complexity: ['error', 25],
      'max-lines-per-function': ['error', { max: 120, skipBlankLines: true, skipComments: true }],
    },
  },
  {
    files: ['**/*.test.ts'],
    rules: { 'max-lines-per-function': 'off' },
  },
);
