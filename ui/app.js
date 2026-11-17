'use strict';
const invoke = window.__TAURI__?.core?.invoke ?? (() => Promise.reject('Open this inside the Lightline app'));
const $ = (s) => document.querySelector(s);
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const fmt = (s) => `${Math.floor(s / 60)}:${(s % 60).toFixed(1).padStart(4, '0')}`;
const EFFECTS = ['Solid', 'Gradient', 'Chase', 'Pulse', 'Rainbow', 'Strobe', 'Bass hit', 'Spectrum'];
const AUDIO_EXT = /\.(mp3|wav|flac|ogg|m4a|aac)$/i;
const ICON = {
  play: '<svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true"><path d="M7 4l13 8-13 8z" fill="currentColor"/></svg>',
  pause: '<svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 4h4v16H6zM14 4h4v16h-4z" fill="currentColor"/></svg>',
  prev: '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M6 5v14M19 5L9 12l10 7z"/></svg>',
  next: '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="M18 5v14M5 5l10 7-10 7z"/></svg>',
};

const demo = () => ({ name: 'Demo scene', len: 60, bpm: 120, tracks: [
  { name: 'Strip · Zone A', segs: 15, clips: [{ fx: 'Solid', start: 0, len: 8, hue: 30 }, { fx: 'Rainbow', start: 16, len: 20, hue: 0 }] },
  { name: 'Strip · Zone B', segs: 15, clips: [{ fx: 'Chase', start: 4, len: 16, hue: 190 }, { fx: 'Pulse', start: 30, len: 14, hue: 320 }] },
  { name: 'Neon', segs: 12, clips: [{ fx: 'Gradient', start: 0, len: 24, hue: 260 }, { fx: 'Strobe', start: 40, len: 4, hue: 0 }] },
] });

const S = { page: 'home', devices: [], scanning: false, msg: '', scenes: [demo()], cur: 0, sel: null, t: 0, playing: false, snap: true, bri: 100, black: false, noSave: false, an: null, anPath: '', loading: '', zoom: 1, ti: 0 };
const sc = () => S.scenes[S.cur];
const beat = () => 60 / (S.an?.bpm || sc().bpm);
// Fractional beat index at time t, from the detected beat grid when there is one.
function beatPos(t) {
  const b = S.an?.beats;
  if (!b || b.length < 2) return t / beat();
  let lo = 0, hi = b.length - 1;
  if (t <= b[0]) return (t - b[0]) / (b[1] - b[0]);
  if (t >= b[hi]) return hi + (t - b[hi]) / (b[hi] - b[hi - 1]);
  while (hi - lo > 1) { const m = (lo + hi) >> 1; if (b[m] <= t) lo = m; else hi = m; }
  return lo + (t - b[lo]) / (b[hi] - b[lo]);
}
function snap(s) {
  if (!S.snap) return s;
  const b = S.an?.beats;
  if (!b?.length) return Math.round(s / beat()) * beat();
  return b[Math.max(0, Math.min(b.length - 1, Math.round(beatPos(s))))];
}
const band = (t, k) => (S.an ? (S.an.bands[Math.floor(t * S.an.fps) * 3 + k] ?? 0) / 255 : 0.5);
const clip = () => S.sel && sc().tracks[S.sel.ti]?.clips[S.sel.ci];
const pct = (s) => `${(s / sc().len) * 100}%`;
const EMPTY = { segs: 12, clips: [] };

// HSL (deg, %, %) to [r, g, b] 0..255.
function hsl(h, s, l) {
  h = ((h % 360) + 360) % 360; s /= 100; l /= 100;
  const a = s * Math.min(l, 1 - l), f = (n) => { const k = (n + h / 30) % 12; return Math.round(255 * (l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1)))); };
  return [f(0), f(8), f(4)];
}

// shortcut: colours are computed in JS for both preview and lights; move to Rust if 30 fps ever costs too much CPU.
function colorsAt(tr, t) {
  const n = tr.segs, c = !S.black && tr.clips.find((k) => t >= k.start && t < k.start + k.len);
  if (!c) return Array(n).fill(null);
  const p = t - c.start, b = beatPos(t) - beatPos(c.start);
  return Array.from({ length: n }, (_, i) => {
    switch (c.fx) {
      case 'Gradient': return hsl(c.hue + (i * 90) / n, 85, 58);
      case 'Chase': return hsl(c.hue, 85, (i - Math.floor(b * 4)) % 4 === 0 ? 58 : 14);
      case 'Pulse': return hsl(c.hue, 85, 14 + 44 * (1 - (((b % 1) + 1) % 1)));
      case 'Rainbow': return hsl(c.hue + i * 24 + p * 90, 85, 58);
      case 'Strobe': return Math.floor(b * 4) % 2 ? [0, 0, 0] : [255, 255, 255];
      case 'Bass hit': return hsl(c.hue, 85, 8 + 52 * band(t, 0));
      case 'Spectrum': { const k = Math.min(2, Math.floor((i * 3) / n)); return hsl(c.hue + k * 50, 85, 8 + 52 * band(t, k)); }
      default: return hsl(c.hue, 85, 58);
    }
  });
}
const css = (c) => (typeof c === 'string' ? c : `rgb(${c[0]} ${c[1]} ${c[2]})`);
const segs = (cols) => cols.map((c) => `<i${c ? ` style="background:${css(c)};color:${css(c)}"` : ''}></i>`).join('');

// Tracks on the same light are chained in track order -> one frame per light.
function deviceFrames() {
  const by = new Map(), k = S.bri / 100;
  for (const tr of sc().tracks) {
    const d = tr.dev && S.devices.find((x) => x.device === tr.dev);
    if (!d) continue;
    const f = by.get(d.ip) ?? [];
    for (const c of colorsAt(tr, S.t)) f.push(...(c ? c.map((v) => Math.round(v * k)) : [0, 0, 0]));
    by.set(d.ip, f);
  }
  return [...by];
}
let sending = false, streamIps = [];
function sendFrame() {
  const frames = deviceFrames();
  if (sending || !frames.length) return; // never queue frames up behind a slow send
  sending = true;
  invoke('govee_frame', { frames }).catch((err) => { $('#status').textContent = String(err); }).finally(() => { sending = false; });
}
function stream(on) {
  const ips = on ? [...new Set(deviceFrames().map(([ip]) => ip))] : streamIps;
  streamIps = on ? ips : [];
  if (ips.length) invoke('govee_stream', { ips, on }).catch((err) => { $('#status').textContent = String(err); });
}
const stage = () => sc().tracks.map((tr) => `<div class="pv"><span class="muted small">${esc(tr.name)}</span><div class="segs">${segs(colorsAt(tr, S.t))}</div></div>`).join('') || '<p class="muted">No tracks yet.</p>';

const V = {
  home: () => `<section class="page">
  <div class="head"><div><h1>Your lights</h1><p class="muted">Build scenes on a timeline, sync them to music, play them live.</p></div>
    <div class="row"><button class="btn" data-act="scan" ${S.scanning ? 'disabled' : ''}>${S.scanning ? 'Scanning…' : 'Scan network'}</button></div></div>
  <h2 class="label">Devices</h2>
  <div class="grid">${S.devices.map((d) => `<article class="card"><div class="split"><div><strong>${esc(d.sku)}</strong><div class="muted small">${esc(d.ip)}</div></div><span class="small muted"><i class="dot"></i>Online</span></div><div class="segs">${segs(Array(12).fill(''))}</div><button class="btn" data-act="identify" data-ip="${esc(d.ip)}">Identify (blink)</button></article>`).join('')
    || `<article class="card"><span class="muted">${esc(S.msg || (S.scanning ? 'Looking for lights…' : 'No lights found. Turn on LAN Control for each light in the Govee Home app, then scan again.'))}</span></article>`}</div>
  <h2 class="label">Scenes</h2>
  <div class="grid"><button class="card newtile" data-act="newScene"><span class="plus" aria-hidden="true">+</span>New scene</button>${S.scenes.map((s, i) => `<article class="card"><div class="segs thumb">${segs(colorsAt(s.tracks[0] ?? EMPTY, s.len / 3))}</div>
    <div class="split"><div><strong>${esc(s.name)}</strong><div class="muted small">${s.tracks.length} tracks · ${fmt(s.len)}</div></div>
    <div class="row"><button class="btn" data-act="open" data-i="${i}">Edit</button><button class="btn primary icon" data-act="pick" data-i="${i}" aria-label="Play ${esc(s.name)}">${ICON.play}</button></div></div></article>`).join('')}</div>
</section>`,

  editor: () => { const s = sc(), c = clip(); return `<div class="editor">
  <div class="tools">
    <input class="title" aria-label="Scene name" value="${esc(s.name)}" data-in="name">
    <label class="muted">Length <input class="num" type="number" min="5" step="1" value="${s.len}" data-in="len"> s</label>
    ${S.an ? '' : `<label class="muted">BPM <input class="num" type="number" min="30" max="300" value="${s.bpm}" data-in="bpm"></label>`}
    <button class="btn" data-act="music">${s.audio ? `♪ ${esc(s.audio.name)}` : '+ Import music'}</button>
    ${S.loading ? `<span class="muted small">${esc(S.loading)}</span>` : S.an ? `<span class="pill"><i class="beatdot" id="beat"></i>${S.an.bpm} BPM · ${S.an.beats.length} beats</span>` : ''}
    <div class="row" style="margin:0 auto"><button class="btn icon" data-act="rewind" aria-label="Back to start">${ICON.prev}</button><button class="btn primary icon" data-act="play" aria-label="${S.playing ? 'Pause' : 'Play'}">${S.playing ? ICON.pause : ICON.play}</button><span id="time" class="mono">${fmt(S.t)} <span class="muted">/ ${fmt(s.len)}</span></span></div>
    <button class="btn" data-act="pick" data-i="${S.cur}">Go live</button>
  </div>
  <div class="panes">
    <aside class="lib"><h2 class="label">Effects</h2><div class="fxgrid">${EFFECTS.map((fx, i) => `<button class="fxtile" data-act="add" data-fx="${fx}"><span class="segs">${segs(Array.from({ length: 6 }, (_, k) => `hsl(${i * 60 + k * 18} 85% 60%)`))}</span>${fx}</button>`).join('')}</div><p class="muted small">Click an effect to drop it at the playhead.</p></aside>
    <section class="stage" id="stage" aria-label="Preview">${stage()}</section>
    <aside class="insp">${c ? inspector(c) : '<h2 class="label">Clip</h2><p class="muted small">Select a clip on the timeline.</p>'}${trackPanel()}</aside>
  </div>
  <section class="tl" aria-label="Timeline">
    <div class="tlhead"><button class="btn" data-act="addTrack">+ Track</button><label class="row muted"><input type="checkbox" data-in="snap" ${S.snap ? 'checked' : ''}>Snap to beat</label>
      <span class="muted small" style="margin-left:auto">Ctrl + scroll to zoom</span>
      <button class="btn icon" data-act="zoom" data-z="-1" aria-label="Zoom out">−</button><input id="zoom" type="range" min="0" max="6" step="0.05" value="${Math.log2(S.zoom)}" data-in="zoom" aria-label="Zoom"><button class="btn icon" data-act="zoom" data-z="1" aria-label="Zoom in">+</button><button class="btn" data-act="fit">Fit</button></div>
    <div id="tl">${timeline()}</div>
  </section>
</div>`; },

  live: () => { const s = sc(); return `<section class="live"><div class="main">
  <div><span class="pill"><i class="beatdot" id="beat"></i>${S.playing ? 'Live' : 'Paused'}</span><h1 class="big">${esc(s.name)}</h1><span class="muted">${s.tracks.length} tracks · ${S.an?.bpm || s.bpm} BPM${s.audio ? ` · ♪ ${esc(s.audio.name)}` : ''}${S.loading ? ` · ${esc(S.loading)}` : ''}</span></div>
  <div class="card stage" id="stage" aria-label="Preview" style="filter:brightness(${S.bri}%)">${stage()}</div>
  <div class="progress"><span id="tnow">${fmt(S.t)}</span><div class="bar2" data-seek><div id="prog" style="width:${pct(S.t)}"></div></div><span>${fmt(s.len)}</span></div>
  <div class="transport"><button class="round" data-act="step" data-d="-1" aria-label="Previous scene">${ICON.prev}</button><button class="round play" data-act="play" aria-label="${S.playing ? 'Pause' : 'Play'}">${S.playing ? ICON.pause : ICON.play}</button><button class="round" data-act="step" data-d="1" aria-label="Next scene">${ICON.next}</button></div>
</div>
<aside>
  <div class="card"><h2 class="label">Scenes</h2>${S.scenes.map((q, i) => `<button class="q" data-act="pick" data-i="${i}" aria-current="${i === S.cur}"><span class="segs">${segs(colorsAt(q.tracks[0] ?? EMPTY, q.len / 3).slice(0, 6))}</span><span><strong>${esc(q.name)}</strong><br><span class="muted small">${fmt(q.len)}</span></span></button>`).join('')}</div>
  <div class="card"><h2 class="label">Controls</h2>
    <label class="field"><span>Master brightness · <b id="briv">${S.bri}</b>%</span><input type="range" min="0" max="100" value="${S.bri}" data-in="bri"></label>
    <button class="btn" data-act="black" aria-pressed="${S.black}">${S.black ? 'Lights back on' : 'Blackout'}</button>
  </div>
</aside></section>`; },
};

const inspector = (c) => `<h2 class="label">Clip</h2><div><strong style="font-size:16px">${c.fx}</strong><div class="muted small">${esc(sc().tracks[S.sel.ti].name)}</div></div>
<label class="field">Effect<select data-in="fx">${EFFECTS.map((f) => `<option${f === c.fx ? ' selected' : ''}>${f}</option>`).join('')}</select></label>
<label class="field">Colour<input type="range" min="0" max="359" value="${c.hue}" data-in="hue"></label><div class="swatch" id="swatch" style="background:hsl(${c.hue} 85% 58%)"></div>
<div class="row"><label class="field" style="flex:1">Start (s)<input type="number" min="0" step="0.1" value="${+c.start.toFixed(2)}" data-in="start"></label><label class="field" style="flex:1">Length (s)<input type="number" min="0.1" step="0.1" value="${+c.len.toFixed(2)}" data-in="clen"></label></div>
<button class="btn" data-act="del">Delete clip</button>`;

const trackPanel = () => {
  const tr = sc().tracks[S.ti]; if (!tr) return '';
  const offline = tr.dev && !S.devices.some((d) => d.device === tr.dev);
  return `<h2 class="label" style="margin-top:8px">Track · ${esc(tr.name)}</h2>
<label class="field">Light<select data-in="tdev"><option value="">None (preview only)</option>${S.devices.map((d) => `<option value="${esc(d.device)}"${d.device === tr.dev ? ' selected' : ''}>${esc(d.sku)} · ${esc(d.ip)}</option>`).join('')}${offline ? `<option value="${esc(tr.dev)}" selected>Light not found</option>` : ''}</select></label>
<label class="field">Segments<input type="number" min="1" max="84" value="${tr.segs}" data-in="tsegs"></label>
<p class="muted small">Tracks on the same light are chained in order: three 5-segment tracks = three zones of a 15-segment strip.</p>`;
};

function timeline() {
  const s = sc(), px = ((($('#tl')?.clientWidth || 1000) - 192) * S.zoom) / s.len;
  const step = [0.5, 1, 2, 5, 10, 15, 30, 60].find((x) => x * px >= 70) ?? 60;
  const ticks = Array.from({ length: Math.ceil(s.len / step) }, (_, i) => `<span style="left:${pct(i * step)}">${step < 1 ? fmt(i * step) : fmt(i * step).slice(0, -2)}</span>`).join('');
  return `<div class="tlbody" style="width:calc(192px + (100% - 192px) * ${S.zoom})"><div class="ruler" data-seek>${ticks}</div>
${S.an ? `<div class="track audio"><div class="thead"><i style="background:var(--accent2)"></i><span class="tname">${esc(s.audio.name)}</span></div><div class="lane" data-seek><canvas id="wave" aria-label="Music waveform with beat lines"></canvas></div></div>` : ''}
${s.tracks.map((tr, ti) => `<div class="track"><div class="thead${ti === S.ti ? ' on' : ''}"><button class="tdot" data-act="track" data-ti="${ti}" aria-label="Track settings" title="Track settings" style="background:hsl(${160 + ti * 70} 70% 60%)"></button><input class="tname" aria-label="Track name" value="${esc(tr.name)}" data-in="tname" data-ti="${ti}"></div><div class="lane" data-seek>${tr.clips.map((c, ci) => `<button class="clip${S.sel?.ti === ti && S.sel?.ci === ci ? ' sel' : ''}" data-act="sel" data-ti="${ti}" data-ci="${ci}" style="left:${pct(c.start)};width:${pct(c.len)};background:hsl(${c.hue} 70% 62%)">${c.fx}</button>`).join('')}</div></div>`).join('')}
<div class="phwrap"><div id="playhead" style="left:${pct(S.t)}"></div></div></div>`;
}

function render() {
  $('#nav').innerHTML = ['home', 'editor', 'live'].map((p) => `<button data-act="go" data-page="${p}"${S.page === p ? ' aria-current="page"' : ''}>${p}</button>`).join('');
  $('#status').textContent = S.noSave ? 'Saving is off: scenes.json could not be read' : `${S.devices.length} light${S.devices.length === 1 ? '' : 's'} found`;
  const sl = $('#tl')?.scrollLeft;
  $('#view').innerHTML = V[S.page]();
  if (sl && $('#tl')) $('#tl').scrollLeft = sl;
  drawWave();
}

// Zooms the timeline keeping the time under clientX (default: the middle) in place.
function setZoom(z, clientX) {
  const tl = $('#tl'); if (!tl) return;
  const r = tl.querySelector('.lane')?.getBoundingClientRect();
  const x = clientX ?? tl.getBoundingClientRect().left + 180 + (tl.clientWidth - 192) / 2;
  const frac = r ? (x - r.left) / r.width : 0;
  S.zoom = Math.max(1, Math.min(64, z));
  tl.innerHTML = timeline();
  const r2 = tl.querySelector('.lane')?.getBoundingClientRect();
  if (r2) tl.scrollLeft += r2.left + frac * r2.width - x;
  $('#zoom').value = Math.log2(S.zoom);
  drawWave();
}

// Bass (purple), mids (teal), highs (white) mirrored around the centre; downbeats as lines, beats as ticks.
// The canvas only covers the visible part of the lane, so zooming never makes a huge canvas.
function drawWave() {
  const cv = $('#wave'), tl = $('#tl'); if (!cv || !tl || !S.an) return;
  const W = cv.parentElement.clientWidth, h = cv.parentElement.clientHeight, dpr = devicePixelRatio || 1;
  const x0 = Math.min(tl.scrollLeft, W - 1), w = Math.max(1, Math.min(W - x0, tl.clientWidth - 192));
  cv.style.left = `${x0}px`; cv.style.width = `${w}px`;
  cv.width = w * dpr; cv.height = h * dpr;
  const g = cv.getContext('2d'); g.scale(dpr, dpr);
  const { bands, fps, beats, downbeats } = S.an, len = sc().len, n = bands.length / 3;
  const X = (t) => (t / len) * W - x0;
  const cols = ['#7c6cff', '#2fd8b4', '#eef0f6'], scale = [0.48, 0.34, 0.2];
  for (let x = 0; x < w; x += 2) {
    const f0 = Math.floor(((x0 + x) / W) * len * fps), f1 = Math.min(n, Math.floor(((x0 + x + 2) / W) * len * fps) + 1);
    for (let k = 0; k < 3; k++) {
      let m = 0; for (let f = f0; f < f1; f++) m = Math.max(m, bands[f * 3 + k]);
      const a = (m / 255) * h * scale[k];
      g.fillStyle = cols[k]; g.fillRect(x, h / 2 - a, 1.5, a * 2);
    }
  }
  g.fillStyle = 'rgba(255,255,255,.55)';
  for (const t of beats) { const x = X(t); if (x >= 0 && x <= w) g.fillRect(x, h - (S.zoom > 3 ? 12 : 5), 1, h); }
  g.fillStyle = 'rgba(255,194,75,.7)';
  for (const t of downbeats) { const x = X(t); if (x >= 0 && x <= w) g.fillRect(x, 0, 1, h); }
}

// Cheap per-frame update: only the parts that move.
function paint() {
  const st = $('#stage'); if (st) st.innerHTML = stage();
  const ph = $('#playhead'); if (ph) ph.style.left = pct(S.t);
  const tm = $('#time'); if (tm) tm.firstChild.nodeValue = `${fmt(S.t)} `;
  const tn = $('#tnow'); if (tn) tn.textContent = fmt(S.t);
  const pr = $('#prog'); if (pr) pr.style.width = pct(S.t);
  const bd = $('#beat'); if (bd) bd.style.opacity = 1 - (((beatPos(S.t) % 1) + 1) % 1);
  const tl = $('#tl');
  if (tl && S.playing && S.zoom > 1) { // keep the playhead in view
    const x = (S.t / sc().len) * (tl.scrollWidth - 192);
    if (x < tl.scrollLeft || x > tl.scrollLeft + tl.clientWidth - 240) tl.scrollLeft = x - 40;
  }
}

let timer = 0, last = 0;
async function setPlaying(on) {
  S.playing = on;
  clearInterval(timer);
  if (on) {
    if (S.t >= sc().len) S.t = 0;
    if (S.an) {
      try { await invoke('audio_play', { at: S.t }); } catch (err) { S.playing = false; $('#status').textContent = String(err); return render(); }
    }
    stream(true);
    last = performance.now();
    let lastSend = 0;
    timer = setInterval(async () => { // ~30 fps, not the monitor's refresh rate
      const now = performance.now();
      if (now - lastSend >= 50) { lastSend = now; sendFrame(); } // lights get ~20 fps
      if (S.an) { // the audio player is the clock
        const [t, playing] = await invoke('audio_state');
        if (!S.playing) return;
        S.t = Math.min(t, sc().len);
        if (!playing) return setPlaying(false);
      } else {
        S.t = Math.min(sc().len, S.t + (now - last) / 1000);
        last = now;
      }
      if (S.t >= sc().len) setPlaying(false); else paint();
    }, 33);
  } else {
    stream(false);
    if (S.an) invoke('audio_pause');
  }
  render();
}

const seek = (t) => { S.t = t; if (S.an) invoke('audio_seek', { at: t }); paint(); };

// Loads (decodes + analyses) the current scene's song unless it is already loaded.
async function ensureAudio() {
  const a = sc().audio;
  if (!a) { S.an = null; return true; }
  if (S.an && S.anPath === a.path) return true;
  S.an = null; S.loading = 'Analysing music…'; render();
  try { S.an = await invoke('load_audio', { path: a.path }); S.anPath = a.path; S.loading = ''; }
  catch (err) { S.loading = `Couldn't load music: ${err}`; }
  render();
  return !!S.an;
}

async function importAudio(path) {
  if (S.playing) await setPlaying(false);
  const s = sc(), old = s.audio;
  s.audio = { path, name: path.split(/[\\/]/).pop() };
  if (!(await ensureAudio())) { s.audio = old; return; }
  s.len = Math.ceil(S.an.duration); s.bpm = Math.round(S.an.bpm);
  save(); render();
}

let saveT = 0;
const save = () => {
  if (S.noSave) return;
  clearTimeout(saveT);
  saveT = setTimeout(() => invoke('save_scenes', { json: JSON.stringify(S.scenes) }).catch((err) => { $('#status').textContent = `Save failed: ${err}`; }), 400);
};

function open(i, page = 'editor') {
  clearInterval(timer);
  if (S.playing) { stream(false); if (S.an) invoke('audio_pause'); }
  Object.assign(S, { cur: i, t: 0, sel: null, ti: 0, page, playing: false });
}

async function scan() {
  S.scanning = true; S.msg = ''; render();
  try { S.devices = await invoke('scan'); } catch (err) { S.msg = String(err); }
  S.scanning = false; render();
}

const A = {
  go: (d) => { S.page = d.page; render(); },
  scan,
  newScene: () => {
    const tracks = S.devices.length ? S.devices.map((d) => ({ name: d.sku, segs: 15, clips: [] })) : [{ name: 'Track 1', segs: 15, clips: [] }];
    S.scenes.push({ name: `Scene ${S.scenes.length + 1}`, len: 60, bpm: 120, tracks });
    open(S.scenes.length - 1); save(); render();
  },
  open: (d) => { open(+d.i); render(); ensureAudio(); },
  pick: async (d) => { open(+d.i, 'live'); render(); if (await ensureAudio()) setPlaying(true); },
  music: async () => { const p = await invoke('pick_audio'); if (p) importAudio(p); },
  step: (d) => A.pick({ i: (S.cur + +d.d + S.scenes.length) % S.scenes.length }),
  play: () => setPlaying(!S.playing),
  rewind: () => seek(0),
  add: (d) => {
    const s = sc(); if (!s.tracks.length) return;
    const ti = S.sel?.ti ?? 0, len = Math.min(4 * beat(), s.len);
    s.tracks[ti].clips.push({ fx: d.fx, start: Math.min(snap(S.t), s.len - len), len, hue: Math.floor(Math.random() * 360) });
    S.sel = { ti, ci: s.tracks[ti].clips.length - 1 };
    save(); render();
  },
  addTrack: () => { sc().tracks.push({ name: `Track ${sc().tracks.length + 1}`, segs: 15, clips: [] }); save(); render(); },
  sel: (d) => { S.sel = { ti: +d.ti, ci: +d.ci }; S.ti = +d.ti; render(); },
  track: (d) => { S.ti = +d.ti; S.sel = null; render(); },
  zoom: (d) => setZoom(S.zoom * (+d.z > 0 ? 1.5 : 1 / 1.5)),
  fit: () => setZoom(1),
  identify: (d) => invoke('govee_identify', { ip: d.ip }),
  del: () => { if (!clip()) return; sc().tracks[S.sel.ti].clips.splice(S.sel.ci, 1); S.sel = null; save(); render(); },
  black: () => { S.black = !S.black; render(); },
};

document.addEventListener('click', (e) => {
  const el = e.target.closest('[data-act]');
  if (el && A[el.dataset.act]) A[el.dataset.act](el.dataset);
});

document.addEventListener('input', (e) => {
  const el = e.target, k = el.dataset.in; if (!k) return;
  const s = sc(), c = clip();
  const v = el.type === 'checkbox' ? el.checked : el.type === 'range' || el.type === 'number' ? +el.value : el.value;
  if (typeof v === 'number' && (Number.isNaN(v) || el.value === '')) return;
  switch (k) {
    case 'snap': S.snap = v; return;
    case 'zoom': setZoom(2 ** v); return;
    case 'tdev': s.tracks[S.ti].dev = v; save(); return;
    case 'tsegs': if (v < 1 || v > 84) return; s.tracks[S.ti].segs = Math.round(v); paint(); save(); return;
    case 'bri': S.bri = v; $('#briv').textContent = v; $('#stage').style.filter = `brightness(${v}%)`; return;
    case 'name': s.name = v; save(); return;
    case 'tname': s.tracks[+el.dataset.ti].name = v; save(); return;
    case 'fx': c.fx = v; save(); render(); return;
    case 'len': if (v < 5) return; s.len = v; break;
    case 'bpm': if (v < 30 || v > 300) return; s.bpm = v; break;
    case 'hue': c.hue = v; $('#swatch').style.background = `hsl(${v} 85% 58%)`; break;
    case 'start': c.start = Math.max(0, Math.min(v, s.len - c.len)); break;
    case 'clen': c.len = Math.max(0.1, Math.min(v, s.len - c.start)); break;
  }
  $('#tl').innerHTML = timeline();
  drawWave(); paint(); save();
});

document.addEventListener('pointerdown', (e) => {
  if (e.button !== 0) return;
  const el = e.target.closest('.clip');
  if (el) return drag(el, e);
  const bar = e.target.closest('[data-seek]');
  if (!bar) return;
  const r = bar.getBoundingClientRect();
  seek(Math.max(0, Math.min(sc().len, ((e.clientX - r.left) / r.width) * sc().len)));
});

function drag(el, e) {
  const s = sc(), c = s.tracks[+el.dataset.ti].clips[+el.dataset.ci];
  const w = el.parentElement.clientWidth, x0 = e.clientX, s0 = c.start, l0 = c.len;
  const resize = e.clientX > el.getBoundingClientRect().right - 8;
  S.sel = { ti: +el.dataset.ti, ci: +el.dataset.ci };
  el.setPointerCapture(e.pointerId);
  el.onpointermove = (ev) => {
    const dt = ((ev.clientX - x0) / w) * s.len;
    if (resize) { c.len = Math.max(0.1, Math.min(s.len, snap(s0 + l0 + dt)) - c.start); el.style.width = pct(c.len); }
    else { c.start = Math.max(0, Math.min(s.len - c.len, snap(s0 + dt))); el.style.left = pct(c.start); }
  };
  el.onpointerup = () => { el.onpointermove = el.onpointerup = null; save(); render(); };
}

document.addEventListener('keydown', (e) => {
  if (e.target.closest('input, select') || S.page === 'home') return;
  if (e.key === ' ') { e.preventDefault(); A.play(); }
  else if ((e.key === 'Delete' || e.key === 'Backspace') && S.page === 'editor') A.del();
});

invoke('load_scenes')
  .then((j) => { const v = JSON.parse(j); if (Array.isArray(v) && v.length) S.scenes = v; })
  .catch((err) => { if (window.__TAURI__) { S.noSave = true; console.error(err); } }) // never overwrite a file we failed to read
  .finally(() => { render(); scan(); ensureAudio(); });

window.__TAURI__?.event.listen('tauri://drag-drop', (e) => {
  const p = e.payload?.paths?.[0];
  if (p && AUDIO_EXT.test(p) && S.page === 'editor') importAudio(p);
});
addEventListener('resize', drawWave);
document.addEventListener('scroll', (e) => { if (e.target.id === 'tl') drawWave(); }, true);
document.addEventListener('wheel', (e) => {
  if (!e.ctrlKey || !e.target.closest('#tl')) return;
  e.preventDefault(); // stop the webview zooming the whole page
  setZoom(S.zoom * (e.deltaY < 0 ? 1.25 : 0.8), e.clientX);
}, { passive: false });
