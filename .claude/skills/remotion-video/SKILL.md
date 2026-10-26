---
name: remotion-video
description: Motion rules for the AegisFlow product film (harness/demo/film). Use when changing the film's camera, captions, callouts or timing.
---

# AegisFlow film: motion rules

These rules follow Remotion's best-practice skills (a local copy of github.com/remotion-dev/skills
is in `upstream/`, gitignored). The film doesn't use Remotion itself. `harness/demo/film/stage.js`
applies the same ideas to the live console in headless Chrome.

- Every motion is a pure function of the frame time `t`. No CSS transitions or animations in the
  overlay. The console's own CSS animations are seeked to the virtual clock each frame.
- Use springs, not linear tweens. Camera moves use a critically damped spring (no overshoot), and UI
  entrances use a slightly under-damped `pop`. Scale moves geometrically.
- Cut timing to the voiceover. The times in `script.mjs` come from `voiceover_words.json`. A click
  lands on the word that names it.
- Captions stay with the audio. Each chunk shows from its first word, the spoken word is
  highlighted, and the next chunk replaces it.
- Long jumps across the page are cuts (a dip to dark), not fast scrolls.
- Never type a number into a callout. Read it from `artifacts/aegis_phase1_telemetry.json` or patch
  it from the live daemon (`__film.patch`). Anything shortened (the live Nemotron call, the order
  stream) gets an on-screen badge.
- Check the result by extracting frames and looking at them before you ship it.
