import type { Component, JSX } from 'solid-js';
import { Route, Router } from '@solidjs/router';

import { Layout } from '@/components/Layout';
import { Home } from '@/pages/Home';
import { Replays } from '@/pages/Replays';
import { Solvers } from '@/pages/Solvers';
import '@/styles/theme.css';

const Shell: Component<{ children?: JSX.Element }> = (props) => <Layout>{props.children}</Layout>;

export function App() {
  return (
    <Router root={Shell}>
      <Route path="/" component={Home} />
      <Route path="/replays" component={Replays} />
      <Route path="/solvers" component={Solvers} />
      <Route path="*" component={Home} />
    </Router>
  );
}
