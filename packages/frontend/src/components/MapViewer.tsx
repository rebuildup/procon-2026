import type { Component } from 'solid-js';
import { For, Show } from 'solid-js';

import type { GameState } from '@/lib/protocol';

/**
 * Stub `MapViewer` for `ReplayViewer`.
 *
 * The full WebGPU-backed map renderer is delivered as part of issue #47. Until
 * that lands this stub still has to do the one thing `ReplayViewer` depends
 * on: render the supplied `GameState` so QA + visual review can lock the
 * transport/scrub UX. Once #47 ships, swap this implementation for the real
 * renderer and keep the prop signature intact.
 */
export const MapViewer: Component<{ state: GameState | null }> = (props) => {
  return (
    <div class="map-viewer" data-testid="map-viewer">
      <Show
        when={props.state}
        fallback={
          <div class="map-viewer__empty" data-testid="map-viewer-empty">
            <p>No replay loaded.</p>
          </div>
        }
      >
        {(state) => (
          <div class="map-viewer__inner" data-testid="map-viewer-inner">
            <header class="map-viewer__hud">
              <span class="map-viewer__match">{state().matchId.value}</span>
              <span class="map-viewer__turn">turn {state().turn}</span>
            </header>
            <div class="map-viewer__canvas" aria-hidden="true">
              <p class="map-viewer__placeholder">
                Map rendering is delivered via issue #47. Showing scoreboard placeholder for state
                preview.
              </p>
            </div>
            <table class="map-viewer__scores" data-testid="map-viewer-scores">
              <thead>
                <tr>
                  <th scope="col">player</th>
                  <th scope="col">score</th>
                </tr>
              </thead>
              <tbody>
                <For each={state().scores}>
                  {(entry) => (
                    <tr>
                      <td>{entry[0].value}</td>
                      <td>{entry[1]}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        )}
      </Show>
    </div>
  );
};
