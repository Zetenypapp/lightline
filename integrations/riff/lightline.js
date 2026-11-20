/**
 * @name Lightline
 * @author Lightline
 * @version 1.0.0
 * @description Tells Lightline (light shows for Govee lights) what is playing and where, so its scenes play in sync. Only talks to this PC.
 */
const send = () => {
  const t = Riff.getTrack();
  const a = Riff.audio();
  if (!t) return;
  const body = JSON.stringify({
    title: String(t.title || ''),
    artist: String(t.artist || ''),
    pos: a && Number.isFinite(a.currentTime) ? a.currentTime : 0,
    playing: !!Riff.isPlaying(),
  });
  fetch('http://127.0.0.1:47830/riff', { method: 'POST', mode: 'no-cors', body }).catch(() => {});
};
const timer = setInterval(send, 250);
const offs = ['trackchange', 'play', 'pause'].map((e) => Riff.on(e, () => setTimeout(send, 50)));
return { stop() { clearInterval(timer); offs.forEach((off) => off()); } };
