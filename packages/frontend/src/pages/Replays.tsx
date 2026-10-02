import type { Component } from 'solid-js';

import { ReplayViewer } from '@/components/ReplayViewer';
import type { GameState, MatchId, PlayerId, Score } from '@/lib/protocol';
import type { Replay } from '@/stores/replay';
import '@/styles/replay-viewer.css';

function player(value: string): PlayerId {
  return { type: 'player', value };
}

function match(value: string): MatchId {
  return { type: 'match', value };
}

/**
 * Build a tiny inline `Replay` so the viewer can be exercised without a real
 * engine. Five turns, two players, monotonically rising scores. Replace once
 * the replay loader lands.
 */
const FIXTURE_REPLAY: Replay = ((): Replay => {
  const players: ReadonlyArray<PlayerId> = [player('alice'), player('bob')];
  const steps: GameState[] = [];
  for (let turn = 0; turn < 5; turn += 1) {
    const scores: ReadonlyArray<readonly [PlayerId, Score]> = players.map(
      (p, idx): readonly [PlayerId, Score] => [p, turn * 10 + idx],
    );
    steps.push({
      matchId: match('demo-match'),
      turn,
      scores,
    });
  }
  return {
    id: 'demo',
    title: 'Demo match (alice vs bob)',
    steps,
  };
})();

export const Replays: Component = () => (
  <section class="page page--replays" data-testid="page-replays">
    <h2 class="page__title">Replays</h2>
    <p class="page__body">
      Preview of the replay playback surface. Real replays will be loaded from the engine once issue
      #49's loader lands.
    </p>
    <div class="page__preview">
      <ReplayViewer replay={FIXTURE_REPLAY} />
    </div>
  </section>
);
