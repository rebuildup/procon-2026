/**
 * Test-only JSX transform plugin.
 *
 * bun's built-in JSX runtime does not understand Solid's hyperscript output,
 * so .tsx files compiled by bun use `React.createElement(...)` which fails at
 * runtime inside Solid components. This plugin re-runs the JSX through
 * babel-preset-solid (the same transform vite-plugin-solid uses) so .tsx files
 * resolve to template()/insert() calls that solid-js/web can render.
 */
import { readFile } from 'node:fs/promises';
import { plugin } from 'bun';
import babel from '@babel/core';
import solid from 'babel-preset-solid';

plugin({
  name: 'solid-jsx',
  setup(build) {
    build.onLoad({ filter: /\.tsx$/, namespace: 'file' }, async (args) => {
      const source = await readFile(args.path, 'utf8');
      const result = await babel.transformAsync(source, {
        filename: args.path,
        sourceFileName: args.path,
        presets: [[solid, { generate: 'dom', hydratable: false }]],
        plugins: [],
        parserOpts: { plugins: ['jsx', 'typescript'] },
        babelrc: false,
        configFile: false,
      });
      if (!result?.code) {
        throw new Error(`solid-jsx: failed to transform ${args.path}`);
      }
      return { contents: result.code, loader: 'tsx' };
    });
  },
});
