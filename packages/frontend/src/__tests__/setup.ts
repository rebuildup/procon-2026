import { GlobalRegistrator } from '@happy-dom/global-registrator';

GlobalRegistrator.register({
  // happy-dom defaults to about:blank, which would leave the router's initial
  // URL at "blank" and prevent the "/" route from matching. Anchor every test
  // to "/" so route resolution starts from a known state.
  url: 'http://localhost/',
});
