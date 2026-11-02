// The recorded pointer window: Tallycast moves it with the mouse and reports clicks.
import { listen } from './tauri.js';
import { mountFx } from './cursorfx.js';

const q = new URLSearchParams(location.search);
const hex = (v, fallback) => (/^[0-9a-f]{6}$/i.test(v || '') ? `#${v}` : fallback);

const fx = mountFx(document.getElementById('fx'));
fx.set({
  halo: q.get('halo') === '1',
  click: q.get('click') === '1',
  haloColor: hex(q.get('hc'), '#ffd60a'),
  clickColor: hex(q.get('cc'), '#e23b3e'),
});

// Messages: "down left", "up right", "show", "hide".
listen('cursorfx', (msg) => {
  const [kind, button] = String(msg).split(' ');
  if (kind === 'down') fx.down(button);
  else if (kind === 'up') fx.up(button);
  else if (kind === 'show') fx.visible(true);
  else if (kind === 'hide') fx.visible(false);
});
