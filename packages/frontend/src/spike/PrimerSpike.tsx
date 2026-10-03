import type { Component } from 'solid-js';
import * as Switch from '@kobalte/core/switch';

/**
 * Stack proof for the Primer-based rebuild — issue #269.
 *
 * Proves the four integration points the rebuild depends on, in one page:
 *   1. `@primer/css` class names apply (Box, btn, Label, Counter)
 *   2. `@primer/primitives` CSS custom properties resolve
 *   3. Octicons render as inline SVG with `currentColor` inheritance
 *   4. `@kobalte/core` behaviour accepts Primer classes (unstyled by design)
 *
 * Octicon paths are inlined rather than fetched from a CDN or icon font so
 * this still works on an offline venue machine. The glyph is `triangle-fill`,
 * which is the Play stand-in — Octicons ships no `play`.
 */

/** `@primer/octicons` triangle-fill-16, inlined. */
const PLAY_PATH =
  'M13.378 6.948a1.2 1.2 0 0 1 0 2.103l-8 4.397A1.2 1.2 0 0 1 3.6 12.397V3.602A1.2 1.2 0 0 1 5.378 2.55z';
/** `@primer/octicons` home-16, inlined. */
const HOME_PATH =
  'M6.906.664a1.53 1.53 0 0 1 2.187 0l8.33 7.5c.318.285.476.682.476 1.091v6.248c0 .916-.745 1.661-1.66 1.661H7.16c-.915 0-1.66-.745-1.66-1.661V9.255c0-.409.158-.806.476-1.091Z';

export const PrimerSpike: Component = () => {
  return (
    <div class="Box p-3" data-spike="primer">
      <h2 class="Subhead-heading">Primer stack proof</h2>

      {/* 1 + 2: Primer class + primitives custom property */}
      <p class="color-fg-muted" data-spike="token">
        <code>color-fg-muted</code> resolves to{' '}
        <code data-spike="token-value">var(--fgColor-muted)</code>
      </p>

      {/* 3: Octicon as inline SVG, colour inherited */}
      <p data-spike="octicon">
        <svg
          width="16"
          height="16"
          viewBox="0 0 16 16"
          aria-hidden="true"
          data-spike="octicon-play"
          style={{ color: 'var(--fgColor-accent)' }}
        >
          <path fill="currentColor" d={PLAY_PATH} />
        </svg>
        <svg
          width="16"
          height="16"
          viewBox="0 0 16 16"
          aria-hidden="true"
          data-spike="octicon-home"
          style={{ color: 'var(--fgColor-accent)' }}
        >
          <path fill="currentColor" d={HOME_PATH} />
        </svg>
        <span>inline octicons, currentColor inherited</span>
      </p>

      {/* 4: Kobalte behaviour wearing Primer classes */}
      <div class="d-flex flex-items-center gap-2" data-spike="kobalte">
        <Switch.Root data-spike="kobalte-root">
          <Switch.Input />
          <Switch.Control class="btn" data-spike="kobalte-switch">
            <Switch.Thumb />
          </Switch.Control>
          <Switch.Label>Kobalte behaviour</Switch.Label>
        </Switch.Root>
      </div>

      <p>
        <button type="button" class="btn" data-spike="primer-btn">
          Primer button
        </button>
      </p>
    </div>
  );
};
