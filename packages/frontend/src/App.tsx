import type { Component, JSX } from 'solid-js';
import { Route, Router } from '@solidjs/router';

import { PrimerSpike } from '@/spike/PrimerSpike';

/**
 * Route skeleton only. The previous UI (components/, pages/, styles/) has been
 * stripped so the rebuild can start from Primer rather than from the retired
 * `--app-*` token system — see issue #265.
 *
 * Each route renders a placeholder. The screens are reintroduced by the
 * rebuild in the order layout frame → tokens → UI placement.
 */
const Placeholder: Component<{ name: string }> = (props) => <p>{props.name}</p>;

/** Home currently shows the stack proof — issue #269. */
const Home: Component = () => <PrimerSpike />;
const Replays: Component = () => <Placeholder name="Replays" />;
const Solvers: Component = () => <Placeholder name="Solvers" />;
const Live: Component = () => <Placeholder name="Live" />;

const Shell: Component<{ children?: JSX.Element }> = (props) => <main>{props.children}</main>;

export function App() {
  return (
    <Router root={Shell}>
      <Route path="/" component={Home} />
      <Route path="/replays" component={Replays} />
      <Route path="/solvers" component={Solvers} />
      <Route path="/live" component={Live} />
      <Route path="*" component={Home} />
    </Router>
  );
}
