// Cursor halo and click ripples. Used by the recorded pointer window (cursor.html) and
// by the preview pad in the main window, so both look exactly the same.

/** Halo diameter (logical px) per size; must match `halo_diameter` in cursorfx.rs. */
export const HALO_SIZES = { s: 44, m: 60, l: 80 };
/** The effect area is this many halo diameters wide; must match `SPAN` in cursorfx.rs. */
export const SPAN = 2.2;

export const HALO_COLORS = [
  { value: '#ffd60a', name: 'Yellow' },
  { value: '#34c759', name: 'Green' },
  { value: '#4aa8ff', name: 'Blue' },
  { value: '#ff5fa2', name: 'Pink' },
];

export const CLICK_COLORS = [
  { value: '#e23b3e', name: 'Red' },
  { value: '#f5b800', name: 'Amber' },
  { value: '#2a57de', name: 'Blue' },
  { value: '#1f9d55', name: 'Green' },
];

/**
 * Builds the effect inside `el` (which should be positioned and sized by the caller;
 * the effect is centred in it) and returns controls for it.
 */
export function mountFx(el) {
  el.classList.add('cfx');
  el.innerHTML = '<div class="cfx-halo"></div><div class="cfx-held"></div>';
  const halo = el.querySelector('.cfx-halo');
  const held = el.querySelector('.cfx-held');
  const opts = { halo: false, click: false };

  function ring(delay) {
    const r = document.createElement('div');
    r.className = 'cfx-ring';
    el.appendChild(r);
    r.animate(
      [{ transform: 'scale(0.32)', opacity: 1 }, { transform: 'scale(1)', opacity: 0 }],
      { duration: 560, delay, easing: 'cubic-bezier(0.2, 0.7, 0.3, 1)', fill: 'both' },
    ).finished.then(() => r.remove(), () => r.remove());
  }

  function flash() {
    const f = document.createElement('div');
    f.className = 'cfx-flash';
    el.appendChild(f);
    f.animate(
      [{ transform: 'scale(0.3)', opacity: 0.75 }, { transform: 'scale(0.66)', opacity: 0 }],
      { duration: 340, easing: 'ease-out', fill: 'both' },
    ).finished.then(() => f.remove(), () => f.remove());
  }

  return {
    set({ halo: showHalo, click, haloColor, clickColor }) {
      if (showHalo !== undefined) opts.halo = showHalo;
      if (click !== undefined) opts.click = click;
      if (haloColor) el.style.setProperty('--cfx-halo', haloColor);
      if (clickColor) el.style.setProperty('--cfx-click', clickColor);
      halo.hidden = !opts.halo;
      if (!opts.click) held.classList.remove('on');
    },
    /** `button` is "left" or "right"; a right-click shows a double ripple. */
    down(button = 'left') {
      if (opts.halo) {
        halo.animate([{ transform: 'scale(1)' }, { transform: 'scale(0.86)' }, { transform: 'scale(1)' }],
          { duration: 260, easing: 'ease-out' });
      }
      if (!opts.click) return;
      flash();
      ring(0);
      if (button === 'right') ring(120);
      held.classList.add('on');
    },
    up() {
      held.classList.remove('on');
    },
    visible(on) {
      el.classList.toggle('cfx-gone', !on);
    },
  };
}
