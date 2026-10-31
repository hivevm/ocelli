// ESLint for the web interface (ADR-0007): the type check is svelte-check's; ESLint holds the
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
      // A failure is reported, never dropped or read as a value (docs/CONVENTIONS.md): a
      // `.catch` whose handler takes no error cannot report it.
      'no-restricted-syntax': [
        'error',
        {
          selector:
            "CallExpression[callee.property.name='catch'] > :matches(ArrowFunctionExpression, FunctionExpression)[params.length=0]",
          message: 'A failure is reported, never dropped: take the error and report it.',
        },
      ],
    },
  },
  {
    files: ['**/*.test.ts'],
    rules: { 'max-lines-per-function': 'off' },
  },
);
