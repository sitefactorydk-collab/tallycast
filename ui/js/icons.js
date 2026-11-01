// A small, consistent icon set (24px grid, 1.75 stroke), drawn for Tallycast.

const paths = {
  screen: '<rect x="3" y="4" width="18" height="12.5" rx="2"/><path d="M9 20h6M12 16.5V20"/>',
  area: '<path d="M4 8V5.5A1.5 1.5 0 0 1 5.5 4H8M16 4h2.5A1.5 1.5 0 0 1 20 5.5V8M20 16v2.5a1.5 1.5 0 0 1-1.5 1.5H16M8 20H5.5A1.5 1.5 0 0 1 4 18.5V16"/><rect x="8.5" y="8.5" width="7" height="7" rx="1"/>',
  mic: '<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21"/>',
  speaker: '<path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"/><path d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11"/>',
  camera: '<rect x="3" y="6.5" width="12.5" height="11" rx="2.5"/><path d="m15.5 10.5 5-3v9l-5-3"/>',
  film: '<rect x="3.5" y="5" width="17" height="14" rx="2"/><path d="M3.5 9.5h17M3.5 14.5h17M8 5v14M16 5v14"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M12 2.8v2.4M12 18.8v2.4M4.2 7.5l2.1 1.2M17.7 15.3l2.1 1.2M4.2 16.5l2.1-1.2M17.7 8.7l2.1-1.2"/><circle cx="12" cy="12" r="6.6"/>',
  minimize: '<path d="M6 12h12"/>',
  maximize: '<rect x="6" y="6" width="12" height="12" rx="1.5"/>',
  close: '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
  folder: '<path d="M3.5 7.5A1.5 1.5 0 0 1 5 6h4.2l2 2.2H19a1.5 1.5 0 0 1 1.5 1.5v7.8A1.5 1.5 0 0 1 19 19H5a1.5 1.5 0 0 1-1.5-1.5z"/>',
  play: '<path d="M8 5.5v13l10.5-6.5z"/>',
  reveal: '<path d="M3.5 7.5A1.5 1.5 0 0 1 5 6h4.2l2 2.2H19a1.5 1.5 0 0 1 1.5 1.5v7.8A1.5 1.5 0 0 1 19 19H5a1.5 1.5 0 0 1-1.5-1.5z"/><path d="M10 13.5h5M13 11.5l2 2-2 2"/>',
  trash: '<path d="M4.5 7h15M10 4h4M6.5 7l.8 11.5A1.5 1.5 0 0 0 8.8 20h6.4a1.5 1.5 0 0 0 1.5-1.5L17.5 7M10 11v5M14 11v5"/>',
  pause: '<path d="M9 5.5v13M15 5.5v13"/>',
  resume: '<path d="M8 5.5v13l10.5-6.5z"/>',
  stop: '<rect x="6.5" y="6.5" width="11" height="11" rx="2"/>',
  alert: '<path d="M12 4 2.8 19.5h18.4z"/><path d="M12 10v4.5M12 17.2v.1"/>',
  check: '<path d="m5 12.5 4.5 4.5L19 7.5"/>',
  scissors: '<circle cx="6.5" cy="7" r="2.6"/><circle cx="6.5" cy="17" r="2.6"/><path d="M8.7 8.5 19.5 16.8M8.7 15.5 19.5 7.2"/>',
  undo: '<path d="M8.5 6.5 4 11l4.5 4.5"/><path d="M4 11h10a5 5 0 0 1 0 10h-2.5"/>',
  redo: '<path d="M15.5 6.5 20 11l-4.5 4.5"/><path d="M20 11H10a5 5 0 0 0 0 10h2.5"/>',
  frameBack: '<path d="M7 5.5v13"/><path d="M18 5.8v12.4L10 12z"/>',
  frameFwd: '<path d="M17 5.5v13"/><path d="M6 5.8v12.4L14 12z"/>',
  toStart: '<path d="M5.5 5.5v13"/><path d="M19 5.8v12.4L12.5 12zM12.5 5.8v12.4L6.5 12z"/>',
  toEnd: '<path d="M18.5 5.5v13"/><path d="M5 5.8v12.4l6.5-6.2zM11.5 5.8v12.4l6-6.2z"/>',
  markIn: '<path d="M10.5 4.5h-4v15h4"/><path d="M13 12h6M16.5 9.5 19 12l-2.5 2.5"/>',
  markOut: '<path d="M13.5 4.5h4v15h-4"/><path d="M11 12H5M7.5 9.5 5 12l2.5 2.5"/>',
  zoomIn: '<circle cx="10.5" cy="10.5" r="6.2"/><path d="m15.2 15.2 4.8 4.8M8 10.5h5M10.5 8v5"/>',
  zoomOut: '<circle cx="10.5" cy="10.5" r="6.2"/><path d="m15.2 15.2 4.8 4.8M8 10.5h5"/>',
  export: '<path d="M12 14.5V4M7.8 8.2 12 4l4.2 4.2"/><path d="M5 13v4.5A2.5 2.5 0 0 0 7.5 20h9a2.5 2.5 0 0 0 2.5-2.5V13"/>',
  restore: '<path d="M4.8 12a7.2 7.2 0 1 0 2.1-5.1"/><path d="M4.5 4.2v4.3h4.3"/>',
  chip: '<rect x="6.5" y="6.5" width="11" height="11" rx="2"/><path d="M10 10h4v4h-4zM9.5 3.5v3M14.5 3.5v3M9.5 17.5v3M14.5 17.5v3M3.5 9.5h3M3.5 14.5h3M17.5 9.5h3M17.5 14.5h3"/>',
  copy: '<rect x="8.5" y="8.5" width="11" height="11" rx="2"/><path d="M15.5 8.5V6a1.5 1.5 0 0 0-1.5-1.5H6A1.5 1.5 0 0 0 4.5 6v8A1.5 1.5 0 0 0 6 15.5h2.5"/>',
  pointer: '<path d="M6 4.5 18 11l-5.6 1.6L9.8 18z"/><path d="m12.6 12.8 4.4 5"/>',
  arrow: '<path d="M5 19 18.5 5.5"/><path d="M10 5.5h8.5V14"/>',
  box: '<rect x="4.5" y="6" width="15" height="12" rx="2"/>',
  highlight: '<path d="m13.5 5.5 5 5-7.5 7.5H6v-5z"/><path d="M4 20.5h16"/>',
  text: '<path d="M5.5 7V5.5h13V7M12 5.5v13M9.5 18.5h5"/>',
  blur: '<rect x="4.5" y="4.5" width="15" height="15" rx="2"/><path d="M4.5 9.5h15M4.5 14.5h15M9.5 4.5v15M14.5 4.5v15"/>',
  shot: '<path d="M4 8.5A2 2 0 0 1 6 6.5h2l1.5-2h5l1.5 2h2a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z"/><circle cx="12" cy="12.5" r="3.5"/>',
  annotate: '<path d="M14.5 5.5 18.5 9.5 9 19H5v-4z"/><path d="M12.5 7.5l4 4"/>',
  open: '<path d="M13.5 4.5h6v6M19.5 4.5 11 13"/><path d="M18 13.5v4.5a1.5 1.5 0 0 1-1.5 1.5h-10A1.5 1.5 0 0 1 5 18V8a1.5 1.5 0 0 1 1.5-1.5H11"/>',
  download: '<path d="M12 4.5V15M7.8 10.8 12 15l4.2-4.2"/><path d="M5 15.5v2A2.5 2.5 0 0 0 7.5 20h9a2.5 2.5 0 0 0 2.5-2.5v-2"/>',
  grip: '<circle cx="9" cy="7" r=".9"/><circle cx="15" cy="7" r=".9"/><circle cx="9" cy="12" r=".9"/><circle cx="15" cy="12" r=".9"/><circle cx="9" cy="17" r=".9"/><circle cx="15" cy="17" r=".9"/>',
};

export function icon(name, cls = '') {
  return `<svg class="icon ${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] || ''}</svg>`;
}

/** Replaces every <i data-icon="name"> in `root` with the SVG. */
export function hydrateIcons(root = document) {
  root.querySelectorAll('i[data-icon]').forEach((el) => {
    el.outerHTML = icon(el.dataset.icon, el.className);
  });
}
