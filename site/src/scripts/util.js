export const doc = document.documentElement;
export const $ = (s, r) => (r || document).querySelector(s);
export const $$ = (s, r) => Array.prototype.slice.call((r || document).querySelectorAll(s));
export const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
export const mobileMQ = window.matchMedia('(max-width: 720px)');
export const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
