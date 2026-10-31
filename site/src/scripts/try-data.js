// Lesson content for /try. Used twice: rendered to static HTML on the server (so crawlers and
// no-JS readers get every lesson), and by try.js to drive the simulator.
// `text` fields use `backticks` for key caps. Task `ev` is a simulator event, `when` a state check in try.js.

export const LESSONS = [
  {
    id: 'move',
    title: 'Move around',
    q: 'How do I move between sessions?',
    a: 'Press `[` or `]` to move focus from one tile to the next, and press a number key, `1` to `9`, to switch to that project in the left rail. The focused tile gets an underlined title. Arrow keys and h j k l do not move focus on the real wall, so they do nothing here either.',
    tasks: [
      { text: 'Press `]` to focus the next tile', ev: 'cycle' },
      { text: 'Press `2` to open the api project', ev: 'ws:2' },
      { text: 'Press `1` to go back to web', ev: 'ws:1' },
    ],
    keypad: ['[', ']', '1', '2', '3'],
    allow: [],
    hint: 'Use [ ] and the number keys.',
  },
  {
    id: 'start',
    title: 'Start a session',
    q: 'How do I start a new Claude Code session?',
    a: 'Press `n` to start a session in the focused project. Press `N` to start one in its own git worktree on a fresh branch, so parallel edits never collide. The new tile joins the grid and Claude starts working. Every session lives in tmux, so closing the wall does not stop it.',
    tasks: [
      { text: 'Press `n` to start a session', ev: 'spawn' },
      { text: 'Press `N` (Shift+n) to start one in a worktree', ev: 'spawn-wt' },
    ],
    keypad: ['n', 'N'],
    allow: ['n', 'N'],
    hint: 'Press n, then N.',
  },
  {
    id: 'amber',
    title: 'The amber one',
    q: 'How do I know which session needs me?',
    a: 'The session waiting on you turns amber. Its tile border, its dot in the left rail and the bottom strip all light up, and the strip says someone needs you and counts how many are blocked. Amber is used for nothing else: working is quiet gray, done is green.',
    tasks: [
      { text: 'Wait a few seconds for a session to ask for permission', when: 'amber' },
      { text: 'Press `[` or `]` until the amber tile is focused', when: 'focusAmber' },
    ],
    keypad: ['[', ']'],
    allow: [],
    hint: 'Wait for amber, then use [ ].',
  },
  {
    id: 'jump',
    title: 'Jump in',
    q: 'How do I answer a permission question?',
    a: 'Press `a` to jump to the session that has waited longest and engage it. Your keys now go straight to Claude: press `Enter` to choose 1. Yes. Then press `Ctrl+G` to leave the session and return to the wall. The tile goes back to working, then turns green when it is done.',
    tasks: [
      { text: 'Press `a` to engage the waiting session', ev: 'engage' },
      { text: 'Press `Enter` to answer 1. Yes', ev: 'answer' },
      { text: 'Press `Ctrl+G` to return to the wall', ev: 'disengage' },
      { text: 'Watch the tile turn green when it is done', when: 'doneSince' },
    ],
    keypad: ['a', 'Enter', 'Ctrl+G'],
    allow: ['a', 'Enter'],
    hint: 'Press a, Enter, then Ctrl+G.',
  },
  {
    id: 'queue',
    title: 'The queue',
    q: 'How do I see everything that needs me?',
    a: 'Press `A` (Shift+a) to open the queue. It lists every blocked session across all projects, longest waiting first. Press `j` and `k` to move the selection, `Enter` to jump into it, and `Esc` to close. When nothing is blocked the queue says nothing needs you.',
    tasks: [
      { text: 'Wait for two sessions to ask for permission', when: 'two' },
      { text: 'Press `A` to open the queue', ev: 'queue-open' },
      { text: 'Press `j` to move down the list', ev: 'queue-next' },
      { text: 'Press `Esc` to close it', ev: 'queue-close' },
    ],
    keypad: ['A', 'j', 'k', 'Esc'],
    allow: ['A'],
    hint: 'Press A, j, then Esc.',
  },
  {
    id: 'restart',
    title: 'Restart safely',
    q: 'How do I restart a session without losing my conversation?',
    a: 'Press `r` twice on the focused tile. claude-garage restarts the process and resumes the same conversation, so the transcript stays. `r a` restarts every idle or done session in the project, and `r d` restarts only the daemon while your sessions keep running.',
    tasks: [
      { text: 'Press `r` then `r` again to restart the focused session', ev: 'restart' },
      { text: 'Check the tile: your transcript is still there', when: 'resumed' },
    ],
    keypad: ['r'],
    allow: ['r'],
    hint: 'Press r, then r again.',
  },
  {
    id: 'close',
    title: 'Close a worktree',
    q: 'How do I close a worktree session?',
    a: 'Press `x` twice to close the focused session. For a worktree session a small dialog then asks what to do with the branch: `m` merges it into the base branch, `d d` discards the branch and worktree, and `k` (or `Esc`) keeps both for later.',
    tasks: [
      { text: 'Press `x` once to arm the close', ev: 'close-armed' },
      { text: 'Press `x` again to close the worktree session', ev: 'closed' },
      { text: 'Choose `m` merge, `d d` discard or `k` keep', ev: 'finish' },
    ],
    keypad: ['x', 'm', 'd', 'k'],
    allow: ['x'],
    hint: 'Press x twice, then m, d d or k.',
  },
  {
    id: 'help',
    title: 'Help',
    q: 'How do I see every key?',
    a: 'Press `?` to open the key list. It shows every binding on the wall, in the same order and wording as the real app. Press `?`, `q` or `Esc` to close it. Everything you just practiced is on that list.',
    tasks: [
      { text: 'Press `?` to open the key list', ev: 'help-open' },
      { text: 'Press `Esc` to close it', ev: 'help-close' },
    ],
    keypad: ['?', 'Esc'],
    allow: [],
    hint: 'Press ? then Esc.',
  },
];

export const FINISH = {
  q: 'How do I install claude-garage?',
  a: 'Run `npx claude-garage` in a terminal on macOS or Linux (Windows is in preview). It opens the wall and keeps every Claude Code session alive in tmux. You now know the keys, so start your own sessions with `n`.',
  command: 'npx claude-garage',
};

export const GLOSSARY = [
  ['Working', 'Quiet gray half-circle. Claude is busy and does not need you.'],
  ['Needs input', 'Amber dot. A permission question or a prompt is waiting for you. The only loud state.'],
  ['Done', 'Green check. Fresh completions stay green, then fade.'],
  ['Idle', 'Dim circle. The session is open and waiting for your next prompt.'],
];
