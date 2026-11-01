// /try: a keyboard-driven simulation of the claude-garage wall plus the lesson controller.
// Faithful to wall/src (keymap.rs, ui/help.rs, rail.rs, strip.rs, tile.rs, triage.rs, worktree_finish.rs):
// same keys, glyphs, colour ladder and overlay wording. Everything is scripted; nothing runs claude, tmux or a network call.
import { LESSONS, FINISH } from './try-data.js';
import { BINDINGS } from './try-bindings.js';

const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
const now = () => Date.now();
const FADE_MS = 14000; // the real wall fades done over 2 minutes; the simulator compresses it
const GLYPH = { 'needs-input': '●', working: '◐', done: '✓', idle: '○', restorable: '⟳' };
const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const fmt = (since) => {
  const t = Math.max(0, Math.floor((now() - since) / 1000));
  return t < 3600 ? `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}` : `${Math.floor(t / 3600)}h ${String(Math.floor((t % 3600) / 60)).padStart(2, '0')}m`;
};
const segs = (pct) => { const f = Math.min(4, Math.ceil(pct / 25)); return '▰'.repeat(f) + '▱'.repeat(4 - f); };

/* ------------------------------------------------------------------ transcripts */
const T = (k, t, w = 0) => ({ k, t, wait: reduce ? 0 : w });
const WEB1 = [
  { k: 'tool', t: 'Read(src/checkout/format.ts)' }, { k: 'res', t: 'Read 24 lines' },
  { k: 'tool', t: 'Grep("toFixed")' }, { k: 'res', t: 'Found 2 matches in 1 file' },
  { k: 'say', t: 'Fixing the checkout total rounding drift.' },
  { k: 'tool', t: 'Update(src/checkout/total.ts)' }, { k: 'res', t: 'Updated total.ts (+3 -1)' },
  { k: 'diff', d: ' ', t: '41   const sum = items.reduce(add)' },
  { k: 'diff', d: '-', t: '42 - return Math.round(sum * tax)/100' },
  { k: 'diff', d: '+', t: '42 + const cents = round(sum*tax)' },
  { k: 'diff', d: '+', t: '43 + return cents / 100' },
  { k: 'say', t: 'Now the checkout tests.' },
];
const WEB2 = [
  { k: 'tool', t: 'Read(src/components/PlanCard.tsx)' }, { k: 'res', t: 'Read 58 lines' },
  { k: 'tool', t: 'Bash(git log --oneline -5)' }, { k: 'res', t: '5 commits' },
  { k: 'say', t: 'Rewriting the pricing headline.' },
  { k: 'tool', t: 'Update(src/pages/pricing.tsx)' }, { k: 'res', t: 'Updated pricing.tsx (+2 -2)' },
  { k: 'diff', d: '-', t: '22 - <h1>Plans and pricing</h1>' },
  { k: 'diff', d: '+', t: '22 + <h1>Pay for what you ship</h1>' },
  { k: 'tool', t: 'Bash(pnpm lint src/pages/pricing.tsx)' }, { k: 'res', t: 'No problems found' },
];
const API1 = [
  { k: 'tool', t: 'Read(src/session/middleware.ts)' }, { k: 'res', t: 'Read 91 lines' },
  { k: 'say', t: 'Moving the token check before the rate limiter.' },
  { k: 'tool', t: 'Update(src/session/middleware.ts)' }, { k: 'res', t: 'Updated middleware.ts (+6 -4)' },
];
const API2 = [
  { k: 'tool', t: 'Update(package.json)' }, { k: 'res', t: 'axios ^1.6.8 to ^1.7.2' },
  { k: 'tool', t: 'Bash(pnpm install)' }, { k: 'res', t: 'Done in 4.1s' },
  { k: 'say', t: 'Bumped axios to 1.7. Tests pass.' },
];
const INFRA1 = [
  { k: 'tool', t: 'Bash(terraform plan)' }, { k: 'res', t: 'Plan: 2 to add, 0 to change, 0 to destroy' },
  { k: 'say', t: 'The plan looks safe. Waiting for your next prompt.' },
];
const PERMS = {
  s1: { cmd: 'pnpm vitest run src/checkout', desc: 'Run the checkout test suite' },
  s2: { cmd: 'pnpm lint src/pages/pricing.tsx', desc: 'Lint the pricing page' },
  s3: { cmd: 'npm test -- middleware', desc: 'Run the middleware tests' },
  def: { cmd: 'git status', desc: 'Show the working tree' },
};

/* ------------------------------------------------------------------ world */
let W;
let timers = [];
const bus = { ev: () => {}, say: () => {} };
const later = (ms, fn) => { const id = setTimeout(fn, ms); timers.push(id); };

function mk(id, ws, label, title, status, ago, lines, o = {}) {
  return {
    id, ws, label, title, status, since: now() - ago * 1000, lines: lines.map((l) => ({ ...l })), queue: [],
    perm: null, msg: null, ctx: o.ctx ?? 4, branch: o.branch || null, wt: o.wt || null,
    restarting: false, resumed: false, doneAt: status === 'done' ? now() - 60000 : 0,
  };
}

function resetWorld() {
  timers.forEach(clearTimeout); timers = [];
  const S = {
    s1: mk('s1', 'web', 'claude-1', 'fix checkout total rounding', 'working', 51, WEB1, { ctx: 68 }),
    s2: mk('s2', 'web', 'claude-2', 'pricing page copy', 'working', 134, WEB2, { ctx: 35 }),
    s3: mk('s3', 'api', 'claude-3', 'session middleware to rate limiter', 'working', 200, API1, { ctx: 52 }),
    s4: mk('s4', 'api', 'claude-4', 'bump axios to 1.7', 'done', 0, API2, { ctx: 21 }),
    s5: mk('s5', 'infra', 'claude-5', 'terraform plan', 'idle', 0, INFRA1, { ctx: 12 }),
  };
  W = {
    S, wss: [{ name: 'web', ids: ['s1', 's2'] }, { name: 'api', ids: ['s3', 's4'] }, { name: 'infra', ids: ['s5'] }],
    ws: 0, focus: { web: 's1', api: 's3', infra: 's5' }, eng: null, ov: null, qsel: 0, fin: null,
    armed: null, chord: 0, notice: null, cnt: 5, wtCnt: 0, dirty: true, spin: 0,
  };
}

const curWs = () => W.wss[W.ws];
const focused = () => W.S[W.focus[curWs().name]] || null;
const all = () => Object.values(W.S);
const needs = () => all().filter((s) => s.status === 'needs-input').sort((a, b) => a.since - b.since);
const wsOf = (s) => W.wss.findIndex((w) => w.name === s.ws);
const nm = (s) => s.title;
function notice(text, ms = 3200) { W.notice = { text, exp: now() + ms }; W.dirty = true; }

function setStatus(s, st) {
  s.status = st; s.since = now(); if (st === 'done') s.doneAt = now(); W.dirty = true;
}
function ask(s) {
  const p = PERMS[s.id] || PERMS.def;
  s.perm = { ...p }; s.msg = 'Claude needs your permission to use Bash';
  setStatus(s, 'needs-input');
  bus.say(`${s.ws}, ${nm(s)}, needs your input. The tile, rail dot and strip are amber.`);
  bus.ev('amber');
}
function askAfter(s, ms) { s.queue = s.queue.filter((q) => !q.ask); s.queue.push({ wait: ms, ask: true, cb: ask }); }
function answer(s) {
  const p = s.perm; s.perm = null; s.msg = null; setStatus(s, 'working');
  s.queue = s.queue.filter((q) => !q.ask);
  s.queue.push(T('tool', `Bash(${p.cmd})`, 600), T('res', '5 passed in 1.2s', 700), T('say', 'All checks pass. Nothing else to change.', 900),
    { wait: 1200, cb: (x) => { setStatus(x, 'done'); bus.say(`${x.ws}, ${nm(x)}, is done.`); } });
  bus.say('Answered 1. Yes. The session is working again.'); bus.ev('answer');
}

function pump(dt) {
  for (const s of all()) {
    for (;;) {
      const it = s.queue[0]; if (!it) break;
      if (it.wait > 0) { it.wait -= dt; if (it.wait > 0) break; it.wait = 0; }
      if (it.cb) { s.queue.shift(); it.cb(s); W.dirty = true; continue; }
      if (!it.line) { it.line = { k: it.k, d: it.d, t: '' }; it.pos = 0; s.lines.push(it.line); if (s.lines.length > 60) s.lines.shift(); }
      it.pos = reduce ? it.t.length : it.pos + 3;
      it.line.t = it.t.slice(0, it.pos); W.dirty = true;
      if (it.pos >= it.t.length) { s.queue.shift(); continue; }
      break;
    }
  }
}

/* ------------------------------------------------------------------ actions */
function focusWs(i) {
  if (!W.wss[i]) { notice(`no workspace ${i + 1}`); return; }
  W.ws = i;
  const w = W.wss[i]; const n = w.ids.map((id) => W.S[id]);
  bus.say(`Workspace ${i + 1}, ${w.name}: ${n.length} session${n.length === 1 ? '' : 's'}${n.some((s) => s.status === 'needs-input') ? ', one needs input' : ''}.`);
  bus.ev('ws:' + (i + 1));
}
function cycle(d) {
  const w = curWs(); if (!w.ids.length) return;
  const i = w.ids.indexOf(W.focus[w.name]);
  W.focus[w.name] = w.ids[(i + d + w.ids.length) % w.ids.length];
  const s = focused(); bus.say(`Focused ${nm(s)}, ${s.status.replace('-', ' ')}.`); bus.ev('cycle');
}
function jump(s) {
  W.ws = wsOf(s); W.focus[s.ws] = s.id; W.eng = s.id; W.ov = null;
  bus.say(`Engaged ${s.ws}, ${nm(s)}. Your keys go to Claude. Ctrl+G returns to the wall.`); bus.ev('engage');
}
function spawn(wt) {
  const w = curWs();
  if (w.ids.length >= 6) { notice('this workspace is full in the simulator (6 tiles)'); return; }
  const id = 's' + (++W.cnt); const label = 'claude-' + W.cnt;
  const s = mk(id, w.name, label, 'new session', 'working', 0, []);
  if (wt) {
    const slug = W.wtCnt ? `dark-mode-${W.wtCnt + 1}` : 'dark-mode'; W.wtCnt++;
    s.branch = `garage/${slug}`; s.wt = { branch: s.branch, path: `/Users/me/.garage/worktrees/${w.name}/${slug}`, target: 'main' };
    s.queue.push(T('user', '> add a dark mode toggle', 500), { wait: 0, cb: (x) => { x.title = 'dark mode toggle'; }, },
      T('tool', 'Read(src/components/Header.tsx)', 700), T('res', 'Read 42 lines', 400), T('say', 'Adding a toggle to the header.', 700),
      T('tool', 'Update(src/components/Header.tsx)', 700), T('res', 'Updated Header.tsx (+9 -1)', 500));
  } else {
    s.queue.push(T('user', '> add unit tests for formatPrice', 500), { wait: 0, cb: (x) => { x.title = 'tests for formatPrice'; } },
      T('tool', 'Read(src/checkout/format.ts)', 700), T('res', 'Read 24 lines', 400), T('say', 'Writing tests for the rounding cases.', 700),
      T('tool', 'Write(src/checkout/format.test.ts)', 700), T('res', 'Wrote 31 lines', 500));
  }
  W.S[id] = s; w.ids.push(id); W.focus[w.name] = id;
  bus.say(`Started ${wt ? 'a worktree session' : 'a session'} in ${w.name}. Claude is typing.`);
  bus.ev(wt ? 'spawn-wt' : 'spawn');
}
function closeFocused() {
  const s = focused(); if (!s) return;
  const w = curWs(); const i = w.ids.indexOf(s.id);
  w.ids.splice(i, 1); delete W.S[s.id];
  W.focus[w.name] = w.ids[Math.min(i, w.ids.length - 1)] || null;
  W.armed = null; W.notice = null;
  if (s.wt) {
    W.fin = { ...s.wt, busy: null, dArm: 0, err: null }; W.ov = 'finish';
    bus.say(`Closed ${nm(s)}. Finish worktree: m merges into main, d d discards, k keeps.`);
  } else { notice(`closed ${nm(s)}`); bus.say(`Closed ${nm(s)}.`); }
  bus.ev('closed');
}
function restart(s) {
  s.restarting = true; W.dirty = true; bus.ev('restart'); bus.say(`Restarting ${nm(s)}. Resuming the conversation.`);
  later(reduce ? 900 : 1500, () => {
    if (!W.S[s.id]) return;
    s.restarting = false; s.resumed = true; s.lines.push({ k: 'sys', t: '── resumed: claude --resume, same conversation ──' });
    s.queue.push(T('say', 'Picking up where I left off.', 500));
    if (s.status !== 'needs-input') setStatus(s, 'working');
    W.dirty = true; bus.say('Resumed. The earlier transcript is still on the tile.');
  });
}
function finishKey(k) {
  const f = W.fin; if (!f || f.busy) return;
  const done = (text) => { W.ov = null; W.fin = null; notice(text, 4000); bus.say(text + '.'); bus.ev('finish'); W.dirty = true; };
  if (k === 'm') { f.busy = 'merge'; W.dirty = true; later(reduce ? 200 : 900, () => done(`merged ${f.branch} into ${f.target}`)); }
  else if (k === 'd') {
    if (f.dArm > now()) { f.busy = 'discard'; W.dirty = true; later(reduce ? 200 : 700, () => done(`discarded ${f.branch}`)); }
    else { f.dArm = now() + 3000; bus.say('Press d again to discard the branch and worktree.'); }
  } else if (k === 'k' || k === 'Esc') done(`kept ${f.branch} and its worktree`);
  W.dirty = true;
}
const qrows = () => needs();

function press(k) {
  W.dirty = true;
  if (W.notice && W.notice.exp < now()) W.notice = null;
  if (W.ov === 'help') { if (k === 'Esc' || k === '?' || k === 'q') { W.ov = null; bus.say('Help closed.'); bus.ev('help-close'); } return; }
  if (W.ov === 'queue') {
    const r = qrows();
    if (k === 'Esc') { W.ov = null; bus.say('Queue closed.'); bus.ev('queue-close'); }
    else if (k === 'j' || k === 'ArrowDown') { W.qsel = r.length ? (W.qsel + 1) % r.length : 0; if (r[W.qsel]) bus.say(`Selected ${r[W.qsel].ws}, ${nm(r[W.qsel])}.`); bus.ev('queue-next'); }
    else if (k === 'k' || k === 'ArrowUp') { W.qsel = r.length ? (W.qsel - 1 + r.length) % r.length : 0; bus.ev('queue-prev'); }
    else if (k === 'Enter') { const s = r[Math.min(W.qsel, r.length - 1)]; W.ov = null; if (s) jump(s); }
    return;
  }
  if (W.ov === 'finish') { finishKey(k); return; }
  if (W.eng) {
    const s = W.S[W.eng];
    if (k === 'ctrl+g') { W.eng = null; bus.say('Back on the wall.'); bus.ev('disengage'); }
    else if ((k === 'Enter' || k === '1') && s && s.perm) answer(s);
    else if (s && s.perm && (k === '2' || k === '3')) notice('in this simulator, answer with Enter or 1');
    else notice('keys go to the session. Ctrl+G returns to the wall');
    return;
  }
  if (W.chord) {
    const live = W.chord > now(); W.chord = 0;
    if (live) {
      const s = focused();
      if (k === 'r' && s) restart(s);
      else if (k === 'a') { notice(`restarted idle and done sessions in ${curWs().name}`); bus.say('Restarted idle and done sessions in this workspace.'); }
      else if (k === 'd') { notice('daemon restarted, sessions untouched'); bus.say('Daemon restarted. Sessions untouched.'); }
      return;
    }
  }
  if (W.armed && W.armed.exp < now()) W.armed = null;
  if (/^[1-9]$/.test(k)) return focusWs(+k - 1);
  switch (k) {
    case '[': return cycle(-1);
    case ']': return cycle(1);
    case 'Enter': { const s = focused(); if (s) jump(s); return; }
    case 'a': { const r = needs(); if (r[0]) jump(r[0]); else notice('nothing needs you'); return; }
    case 'A': W.ov = 'queue'; W.qsel = 0; bus.say(`Queue: ${needs().length || 'no'} session${needs().length === 1 ? '' : 's'} waiting, longest first.`); bus.ev('queue-open'); return;
    case 'n': return spawn(false);
    case 'N': return spawn(true);
    case 'x': {
      const s = focused(); if (!s) return;
      if (W.armed && W.armed.id === s.id) return closeFocused();
      W.armed = { id: s.id, exp: now() + 3000 }; notice(`press x again to close ${s.ws}/${s.label}`, 3000);
      bus.say(`Press x again to close ${nm(s)}.`); bus.ev('close-armed'); return;
    }
    case 'r': W.chord = now() + 3000; notice('r: restart this session · a idle/done · d daemon', 3000); bus.say('r pressed. r restarts this session, a restarts idle and done, d restarts the daemon.'); return;
    case '?': W.ov = 'help'; bus.say(`Help: ${BINDINGS.length} keys listed.`); bus.ev('help-open'); return;
    case 'ArrowUp': case 'ArrowDown': case 'ArrowLeft': case 'ArrowRight': case 'h': case 'j': case 'k': case 'l':
      notice('arrows and h j k l do nothing on the wall: use [ ] and 1-9'); bus.ev('arrow'); return;
    default: notice(`${k}: not in this simulation`);
  }
}

/* ------------------------------------------------------------------ rendering */
function glyph(s, gridded) {
  const p = s.status === 'done' ? (reduce ? ((now() - s.doneAt) > FADE_MS ? 100 : 0) : Math.min(100, Math.round(((now() - s.doneAt) / FADE_MS) * 100))) : 0;
  const cls = !gridded && s.status !== 'needs-input' ? 'st-faint' : 'st-' + s.status;
  return `<span class="g ${cls}"${s.status === 'done' ? ` style="--p:${p}%"` : ''}>${GLYPH[s.status]}</span>`;
}
function lineHtml(l) {
  const t = esc(l.t);
  switch (l.k) {
    case 'tool': return `<div class="l tl"><b class="bu">●</b> ${t}</div>`;
    case 'say': return `<div class="l"><b class="bu">●</b> ${t}</div>`;
    case 'res': return `<div class="l res">  ⎿ ${t}</div>`;
    case 'user': return `<div class="l usr">${t}</div>`;
    case 'sys': return `<div class="l sys">${t}</div>`;
    case 'diff': return `<div class="l df ${l.d === '-' ? 'dm' : l.d === '+' ? 'dp' : 'dc'}">    ${t}</div>`;
    default: return `<div class="l">${t}</div>`;
  }
}
function tileHtml(s) {
  const isF = W.focus[s.ws] === s.id; const eng = W.eng === s.id; const bl = s.status === 'needs-input';
  const cls = ['t', bl ? 'bl' : '', eng ? 'eng' : '', isF ? 'foc' : ''].join(' ');
  const el = s.status === 'needs-input' || s.status === 'working' ? ` <span class="el${bl ? ' am' : ''}">${fmt(s.since)}</span>` : '';
  const title = `<div class="tt"> ${glyph(s, true)} <span class="nm${bl ? ' am' : ''}${eng ? ' bo' : ''}${isF ? ' un' : ''}">${esc(s.title)}</span>${s.branch ? ` <span class="br">⎇ ${esc(s.branch)}</span>` : ''}${el} <span class="cx${s.ctx >= 80 ? ' hot' : ''}">${segs(s.ctx)} ${s.ctx}%</span> </div>`;
  let body;
  if (s.restarting) {
    const sp = reduce ? '⟳' : '⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏'[W.spin % 10];
    body = `<div class="ph"><div>${sp} restarting…</div><div class="fa">resuming the conversation</div></div>`;
  } else {
    const lines = s.lines.slice(-16).map(lineHtml).join('');
    let extra = '';
    if (s.perm) {
      extra = `<div class="perm"><div class="pt">Bash command</div><div>  ${esc(s.perm.cmd)}</div><div class="fa">  ${esc(s.perm.desc)}</div><div class="pq">Do you want to proceed?</div><div class="opt on">❯ 1. Yes</div><div class="opt">  2. Yes, and don't ask again for ${esc(s.perm.cmd.split(' ')[0])}</div><div class="opt">  3. No, and tell Claude what to do differently (esc)</div></div>`;
    } else if (s.status === 'working') {
      extra = `<div class="l wk">✻ Working… (${Math.max(0, Math.floor((now() - s.since) / 1000))}s · esc to interrupt)</div>`;
    }
    body = `<div class="tb">${lines}${extra}</div>`;
  }
  return `<div class="${cls}">${title}${body}</div>`;
}
function railHtml() {
  let h = '<div class="rail">';
  W.wss.forEach((w, i) => {
    const foc = i === W.ws; const blocked = w.ids.filter((id) => W.S[id].status === 'needs-input').length;
    h += `<div class="r"><span class="rn"> ${i + 1} </span><span class="rw${foc ? ' fw' : ''}">${esc(w.name)}</span>${blocked ? `<span class="am">  ● ${blocked}</span>` : ''}</div>`;
    for (const id of w.ids) {
      const s = W.S[id]; const grid = foc; const isF = foc && W.focus[w.name] === id; const bl = s.status === 'needs-input';
      const el = bl || s.status === 'working' ? `<span class="el${bl ? ' am' : ''}"> ${fmt(s.since)}</span>` : '';
      h += `<div class="r rs"><span class="mk">${isF ? ' ▸ ' : '   '}</span>${glyph(s, grid)}<span class="rt${bl ? ' am' : grid ? (isF ? ' fg' : ' dm2') : ' fa'}${isF ? ' bo' : ''}"> ${esc(s.title)}</span>${el}</div>`;
    }
  });
  return h + '</div>';
}
function stripHtml() {
  const bl = needs().length;
  let left = '';
  W.wss.forEach((w, i) => { left += `<span class="sw${i === W.ws ? ' fw' : ''}"> ${i + 1}:${esc(w.name)}</span>${w.ids.some((id) => W.S[id].status === 'needs-input') ? '<span class="am">●</span>' : ''}`; });
  const pet = bl ? `<span>someone needs you</span> <span class="pet">(0.0)</span> <span class="am">!</span>` : '<span class="fa">=-.-=</span>';
  const n = W.notice && W.notice.exp > now() ? `<span class="nt"> ${esc(W.notice.text)} </span>` : '';
  const s = W.eng ? W.S[W.eng] : null;
  let chip = 'keys → garage'; let chipCls = '';
  if (W.ov) chip = { help: 'keys → help', queue: 'keys → queue', finish: 'keys → finish worktree' }[W.ov];
  else if (s) { chip = `keys → ${s.ws}/${s.label}`; chipCls = ' eg'; }
  return `<div class="strip"><div class="sl">${left}</div><div class="sp">${pet}</div><div class="sn">${n}<span class="fa us">5h 24% · wk 61%</span>${bl ? `<span class="am"> ● ${bl} blocked </span>` : ''}<span class="chip${chipCls}"> ${esc(chip)} </span></div></div>`;
}
function overlayHtml() {
  if (W.ov === 'help') {
    const rows = BINDINGS.map(([k, w]) => `<div class="hr"><span class="hk">${esc(k)}</span><span class="fa2">${esc(w)}</span></div>`).join('');
    return `<div class="ov hp"><div class="ot">keys</div>${rows}</div>`;
  }
  if (W.ov === 'queue') {
    const r = qrows();
    let body = r.length ? r.map((s, i) => `<div class="qr${i === Math.min(W.qsel, r.length - 1) ? ' sel' : ''}"><span>${i === Math.min(W.qsel, r.length - 1) ? '▸ ' : '  '}</span><span class="am">● </span><span class="qi">${esc(s.ws)}/${esc(s.title)}</span><span class="am">  ${fmt(s.since)}</span><span class="fa">  ${esc(s.msg || '')}</span></div>`).join('') : '<div class="dm2">nothing needs you</div>';
    return `<div class="ov qu"><div class="ot">triage queue</div>${body}<div class="qf fa">j/k move · Enter jump · Esc close</div></div>`;
  }
  if (W.ov === 'finish' && W.fin) {
    const f = W.fin; const idle = !f.busy; const armed = f.dArm > now();
    const row = (k, t) => `<div class="hr"><span class="hk">${k}</span><span class="${idle ? 'dm2' : 'fa'}">${t}</span></div>`;
    let st = '';
    if (f.busy) st = `<div class="dm2">${f.busy === 'merge' ? 'merging…' : 'discarding…'}</div>`;
    else if (armed) st = '<div class="dm2">press d again to discard branch and worktree</div>';
    return `<div class="ov fn"><div class="ot">finish worktree</div><div><b>${esc(f.branch)}</b><span class="dm2"> → </span>${esc(f.target)}</div><div class="fa">${esc(f.path)}</div><div>&nbsp;</div>${row('m', `merge into ${esc(f.target)}`)}${row('d d', 'discard branch and worktree')}${row('k', 'keep both for later (or Esc)')}${st ? '<div>&nbsp;</div>' + st : ''}</div>`;
  }
  return '';
}
function wallHtml() {
  const w = curWs(); const n = w.ids.length;
  const cols = n <= 1 ? 1 : n <= 4 ? 2 : 3; const rows = Math.max(1, Math.ceil(n / cols));
  const tiles = n ? w.ids.map((id) => tileHtml(W.S[id])).join('') : '<div class="ph">no sessions here<div class="fa">press n to start one</div></div>';
  return `<div class="w">${railHtml()}<div class="grid" style="--c:${cols};--rw:${rows}">${tiles}</div>${stripHtml()}${overlayHtml()}</div>`;
}

/* ------------------------------------------------------------------ UI + lessons */
function init() {
  const root = document.getElementById('try');
  const frame = document.getElementById('tr-frame');
  const wall = document.getElementById('tr-wall');
  if (!root || !frame || !wall) return;
  const live = document.getElementById('tr-live');
  const stateEl = document.getElementById('tr-state');
  const pad = document.getElementById('tr-keypad');
  const hintEl = document.getElementById('tr-hint');
  const lis = Array.from(root.querySelectorAll('[data-lesson]'));
  const dots = Array.from(root.querySelectorAll('[data-dot]'));
  root.classList.add('tr-ready');

  const announce = (msg) => {
    const p = document.createElement('p'); p.textContent = msg; live.appendChild(p);
    while (live.children.length > 6) live.firstChild.remove();
  };
  bus.say = announce;
  bus.ev = (e) => onEvent(e);

  let cur = 0; let taskIdx = 0; let lessonDone = false; let free = false; let lstart = now(); let advanceTimer = 0;
  const WHEN = {
    amber: () => needs().length >= 1,
    focusAmber: () => !W.eng && focused() && focused().status === 'needs-input',
    doneSince: () => all().some((s) => s.status === 'done' && s.doneAt > lstart),
    two: () => needs().length >= 2,
    resumed: () => all().some((s) => s.resumed && !s.restarting),
  };

  function calm() {
    W.ov = null; W.fin = null; W.eng = null; W.armed = null; W.chord = 0; W.notice = null;
    for (const s of all()) {
      s.queue = s.queue.filter((q) => !q.ask);
      if (s.status === 'needs-input') { s.perm = null; s.msg = null; setStatus(s, 'working'); }
      s.resumed = false;
    }
  }
  function ensureWt() {
    let s = all().find((x) => x.wt);
    if (!s) {
      const web = W.wss[0];
      s = mk('s' + (++W.cnt), 'web', 'claude-' + W.cnt, 'dark mode toggle', 'working', 40, [
        { k: 'user', t: '> add a dark mode toggle' }, { k: 'tool', t: 'Read(src/components/Header.tsx)' }, { k: 'res', t: 'Read 42 lines' },
        { k: 'say', t: 'Adding a toggle to the header.' },
      ], { branch: 'garage/dark-mode' });
      s.wt = { branch: 'garage/dark-mode', path: '/Users/me/.garage/worktrees/web/dark-mode', target: 'main' };
      W.S[s.id] = s; web.ids.push(s.id);
    }
    return s;
  }
  function setup(i, jumped) {
    if (jumped) resetWorld(); else calm();
    W.ws = 0;
    switch (i) {
      case 0: break;
      case 1: break;
      case 2: W.focus.web = 's2'; if (W.S.s1) { W.S.s1.perm = null; if (W.S.s1.status !== 'working') setStatus(W.S.s1, 'working'); askAfter(W.S.s1, 3500); } break;
      case 3: if (!needs().length && W.S.s1) ask(W.S.s1); break;
      case 4: if (W.S.s3) askAfter(W.S.s3, 2000); if (W.S.s2) askAfter(W.S.s2, 4500); break;
      case 5: { const s = W.S.s1; if (s) { W.focus.web = 's1'; if (s.status !== 'working') setStatus(s, 'working'); } break; }
      case 6: { const s = ensureWt(); W.focus.web = s.id; break; }
      default: break;
    }
    W.dirty = true;
  }

  function renderPanel() {
    lis.forEach((li, i) => {
      const st = i === cur ? 'current' : i < cur || (cur >= 8) ? 'done' : 'todo';
      li.dataset.state = st;
      const done = li.querySelectorAll('[data-task]');
      done.forEach((t, j) => { t.dataset.done = (i < cur || cur >= 8 || (i === cur && j < taskIdx) || (i === cur && lessonDone)) ? '1' : '0'; t.dataset.now = (i === cur && !lessonDone && j === taskIdx) ? '1' : '0'; });
    });
    dots.forEach((d, i) => {
      const done = i < cur || cur >= 8 || (i === cur && lessonDone);
      d.dataset.state = done ? 'done' : i === cur ? 'current' : 'todo';
      if (i === cur && cur < 8) d.setAttribute('aria-current', 'step'); else d.removeAttribute('aria-current');
      d.setAttribute('aria-label', `Lesson ${i + 1}, ${LESSONS[i].title}${done ? ', complete' : i === cur ? ', current' : ''}`);
    });
    const finish = root.querySelector('[data-finish]'); if (finish) finish.dataset.state = cur >= 8 ? 'current' : 'todo';
    root.querySelector('#tr-count').textContent = cur >= 8 ? 'All 8 lessons done' : `Lesson ${cur + 1} of 8`;
    buildPad(); updateHint();
    const panel = root.querySelector('.tr-panel'); const c = cur < 8 ? lis[cur] : root.querySelector('[data-finish]');
    if (c && panel.scrollHeight > panel.clientHeight + 4) panel.scrollTop = Math.max(0, c.offsetTop - ((panel.querySelector('.tr-ph')?.offsetHeight ?? 74) + 10));
  }
  function buildPad() {
    const keys = cur >= 8 ? ['[', ']', 'n', 'a', 'A', '?'] : LESSONS[cur].keypad;
    pad.innerHTML = keys.map((k) => `<button type="button" class="tr-key" data-k="${esc(k === 'Ctrl+G' ? 'ctrl+g' : k)}" aria-label="Press ${esc(k)}"><kbd>${esc(k)}</kbd></button>`).join('');
  }
  function updateHint() {
    if (!hintEl) return;
    const focusedNow = document.activeElement === frame;
    const touch = matchMedia('(hover: none)').matches;
    hintEl.textContent = touch ? 'Tap the keys below to drive the wall.' : focusedNow ? 'Keys are captured by the wall. Press Esc to release them.' : 'Click the wall to focus it, or tap the keys below.';
  }
  function startLesson(i, jumped) {
    clearTimeout(advanceTimer);
    cur = i; taskIdx = 0; lessonDone = false; lstart = now();
    setup(i, jumped);
    renderPanel();
    announce(`Lesson ${i + 1} of 8, ${LESSONS[i].title}. ${LESSONS[i].q} ${LESSONS[i].tasks[0].text.replace(/`/g, '')}.`);
    check();
  }
  function showFinish() {
    cur = 8; free = true; calm(); renderPanel();
    announce('All 8 lessons complete. You can keep pressing keys on the wall. Install with npx claude-garage.');
    const f = root.querySelector('[data-finish]'); if (f && matchMedia('(max-width: 900px)').matches) f.scrollIntoView({ block: 'nearest', behavior: reduce ? 'auto' : 'smooth' });
  }
  function advance() {
    taskIdx++;
    const L = LESSONS[cur];
    if (taskIdx >= L.tasks.length) {
      lessonDone = true; renderPanel(); announce(`Lesson ${cur + 1} complete: ${L.title}.`);
      advanceTimer = setTimeout(() => (cur + 1 < 8 ? startLesson(cur + 1, false) : showFinish()), reduce ? 900 : 1700);
      return;
    }
    renderPanel(); announce(`Done. Next: ${L.tasks[taskIdx].text.replace(/`/g, '')}.`); check();
  }
  function check() {
    if (free || lessonDone || cur >= 8) return;
    const t = LESSONS[cur].tasks[taskIdx];
    if (t && t.when && WHEN[t.when]()) advance();
  }
  function onEvent(e) {
    if (free || lessonDone || cur >= 8) return;
    const t = LESSONS[cur].tasks[taskIdx];
    if (t && t.ev === e) advance();
  }

  const NAV = new Set(['[', ']', '1', '2', '3', '4', '5', '6', '7', '8', '9', '?', 'Esc']);
  function layer() { return W.ov ? 'ov' : W.eng ? 'eng' : W.chord > now() ? 'chord' : 'garage'; }
  function dispatch(k, fromPad) {
    if (k === 'Esc' && !W.ov) { if (!fromPad && document.activeElement === frame) release(); return; }
    if (layer() === 'garage' && !free && !NAV.has(k) && !(cur < 8 && LESSONS[cur].allow.includes(k))) {
      const arrow = k.startsWith('Arrow') || 'hjkl'.includes(k);
      notice(arrow ? 'arrows and h j k l do nothing on the wall: use [ ] and 1-9' : `not needed in this lesson. ${cur < 8 ? LESSONS[cur].hint : ''}`);
      return;
    }
    press(k);
  }
  function release() { frame.blur(); announce('Keyboard released. Page keys and scrolling work again.'); }

  function keyName(e) {
    if (e.ctrlKey && !e.metaKey && !e.altKey && (e.key === 'g' || e.key === 'G')) return 'ctrl+g';
    if (e.ctrlKey || e.metaKey || e.altKey) return null;
    const k = e.key;
    if (k === 'Escape') return 'Esc';
    if (k === 'Enter' || k.startsWith('Arrow')) return k;
    if (k.length === 1 && k !== ' ') return k;
    return null;
  }
  frame.addEventListener('keydown', (e) => {
    const k = keyName(e); if (!k) return;
    if (k === 'Esc' && !W.ov) { e.preventDefault(); release(); return; }
    e.preventDefault(); dispatch(k, false);
  });
  frame.addEventListener('focus', () => { root.classList.add('tr-focus'); stateEl.textContent = 'Keys captured. Esc releases'; updateHint(); });
  frame.addEventListener('blur', () => { root.classList.remove('tr-focus'); stateEl.textContent = 'Click the wall to use your keyboard'; updateHint(); });
  pad.addEventListener('mousedown', (e) => e.preventDefault());
  pad.addEventListener('click', (e) => {
    const b = e.target.closest('[data-k]'); if (!b) return;
    dispatch(b.dataset.k, true);
  });
  dots.forEach((d, i) => d.addEventListener('click', () => { free = false; startLesson(i, !(i === cur + 1 && lessonDone)); }));
  root.querySelector('#tr-skip').addEventListener('click', () => { if (cur < 8) { if (cur + 1 < 8) startLesson(cur + 1, true); else showFinish(); } });
  root.querySelector('#tr-reset').addEventListener('click', () => { free = false; startLesson(0, true); });
  document.querySelectorAll('[data-copy]').forEach((b) => b.addEventListener('click', () => {
    const t = b.getAttribute('data-copy'); const lbl = b.querySelector('.lbl');
    const ok = () => { if (lbl) { lbl.textContent = 'Copied'; setTimeout(() => { lbl.textContent = 'Copy'; }, 1800); } announce('Copied ' + t); };
    if (navigator.clipboard && navigator.clipboard.writeText) navigator.clipboard.writeText(t).then(ok, ok); else ok();
  }));

  // main loop
  let last = performance.now(); let lastSec = 0; let html = '';
  function tick() {
    const t = performance.now(); const dt = t - last; last = t;
    pump(dt);
    if (W.restarting || all().some((s) => s.restarting)) { W.spin++; W.dirty = true; }
    if (W.notice && W.notice.exp < now()) { W.notice = null; W.dirty = true; }
    if (W.chord && W.chord < now()) { W.chord = 0; W.dirty = true; }
    const sec = Math.floor(now() / 1000);
    if (sec !== lastSec) { lastSec = sec; W.dirty = true; }
    check();
    if (W.dirty) {
      W.dirty = false;
      const h = wallHtml();
      if (h !== html) { html = h; wall.innerHTML = h; }
    }
  }
  resetWorld();
  startLesson(0, false);
  tick();
  setInterval(tick, reduce ? 120 : 50);
  window.__try = { press: dispatch, state: () => W }; // used by the headless verification script
}

if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init); else init();
