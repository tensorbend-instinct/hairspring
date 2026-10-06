const T = window.__TAURI__, invoke = T.core.invoke, listen = T.event.listen;
const $ = (h) => { const d = document.createElement('div'); d.innerHTML = h.trim(); return d.firstChild; };
const app = document.getElementById('app');
const traj = []; let t0 = 0, calls = 0, steps = 0, tin = 0, tout = 0, cached = 0, tinCache = 0, ticker = null, asst = null, running = {};
const fmt = (n) => n >= 1000 ? (n / 1000).toFixed(1) + 'K' : '' + n;

async function boot() {
  const s = await invoke('setup_status');
  if (!s.ready) return setup(s);
  await invoke('open_engine').catch((e) => alert(e));
  shell(); refresh(); projectWarn(); wsBar(true);
}

async function projectWarn() {
  const p = await invoke('project_status').catch(() => null), el = document.getElementById('pwarn');
  if (!el || !p) return;
  el.style.display = p.empty ? 'block' : 'none';
  el.textContent = p.empty ? 'Project folder is empty (' + p.root + '). Missions can only see this folder. Set one in Settings > Project folder.' : '';
}

function setup(s) {
  app.innerHTML = '';
  const el = $(`<div id="setup"><h2>Welcome to HAIRSPRING</h2>
    <div>Paste a provider key. It is saved owner-only on this machine.</div>
    <input id="prov" value="deepseek"><input id="key" type="password" placeholder="API key"><button id="save">Save and start</button>
    <small>${s.config_exists ? '' : 'No rig config at ' + s.config + ' yet. Run the installer once to create it.'}</small></div>`);
  app.append(el);
  el.querySelector('#save').onclick = async () => {
    await invoke('save_key', { provider: el.querySelector('#prov').value, key: el.querySelector('#key').value }).catch((e) => alert(e));
    boot();
  };
}

function shell() {
  app.innerHTML = `<div id="side"><div id="wsbar"></div><button class="new" id="new">New session</button><div id="list"></div></div>
  <div id="main"><div id="tabs"><span class="on" data-t="chat">Chat</span><span data-t="traj">Trajectory</span><span data-t="plug">Plugins</span><span data-t="set">Settings</span></div>
  <div id="pwarn"></div><div id="chat"></div><div id="traj"></div><div id="plug"></div><div id="set"></div><div id="qcard"></div><div id="queue"></div>
  <div id="comp"><div id="menu"></div><textarea id="in" rows="2" placeholder="Message or run a task, / commands"></textarea></div>
  <div id="foot"><span id="f1">idle</span><span id="f2"></span><span id="f3"></span></div></div>`;
  document.querySelectorAll('#tabs span').forEach((s) => s.onclick = () => {
    document.querySelectorAll('#tabs span').forEach((x) => x.classList.toggle('on', x === s));
    for (const t of ['chat', 'traj', 'plug', 'set']) document.getElementById(t).style.display = s.dataset.t === t ? (t === 'chat' ? '' : 'block') : 'none';
    if (s.dataset.t === 'traj') showTraj(); if (s.dataset.t === 'plug') showPlugins(); if (s.dataset.t === 'set') showSettings();
  });
  const inp = document.getElementById('in'), menu = document.getElementById('menu');
  inp.oninput = async () => {
    const v = inp.value;
    if (v.startsWith('/') && !v.includes(' ')) {
      const cmds = await invoke('slash_menu', { prefix: v });
      menu.innerHTML = ''; menu.style.display = cmds.length ? 'block' : 'none';
      cmds.forEach((c) => { const r = $(`<div>${c.name}<small>${c.description}</small></div>`); r.onclick = () => { inp.value = c.name + ' '; menu.style.display = 'none'; inp.focus(); }; menu.append(r); });
    } else menu.style.display = 'none';
  };
  inp.onkeydown = (e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(inp.value); inp.value = ''; menu.style.display = 'none'; } };
  document.getElementById('new').onclick = () => { document.getElementById('chat').innerHTML = ''; traj.length = 0; };
}

const age = (s) => s < 60 ? s + 's' : s < 3600 ? Math.floor(s / 60) + 'm' : s < 86400 ? Math.floor(s / 3600) + 'h' : Math.floor(s / 86400) + 'd';
let selected = null;
async function refresh() {
  const s = await invoke('sessions'), list = document.getElementById('list');
  list.innerHTML = '';
  for (const g of s.groups) {
    list.append($(`<div class="ws">${g.workspace.split('/').pop() || g.workspace || '(no workspace)'}</div>`));
    for (const x of g.sessions) {
      const r = $(`<div class="sess"><span class="t"></span><small>${age(x.age_secs)}</small></div>`);
      r.querySelector('.t').textContent = (x.title || x.id.slice(0, 8)).slice(0, 40);
      r.onclick = async () => { selected = x.id; document.querySelectorAll('.sess').forEach((e) => e.classList.toggle('sel', e === r)); const o = await invoke('resume_session', { id: x.id }).catch((e) => ({ error: '' + e })); if (o.error) return addDone('could not open: ' + o.error); addDone('opened in ' + o.workspace); wsBar(); projectWarn(); };
      r.ondblclick = async () => { const p = prompt('Export ZIP to path', '/tmp/' + x.id.slice(0, 8) + '.zip'); if (p) addDone('exported ' + await invoke('export_session', { id: x.id, path: p }).catch((e) => 'error ' + e) + ' files'); };
      list.append(r);
    }
  }
}

async function showTraj() {
  const el = document.getElementById('traj'); el.innerHTML = '';
  if (!selected) { el.textContent = 'Select a session in the sidebar.'; return; }
  const t = await invoke('trajectory', { id: selected }).catch((e) => ({ error: '' + e }));
  if (t.error) { el.textContent = t.error; return; }
  const max = Math.max(t.duration_ms, 1);
  const bar = (label, val, txt) => `<div class="bar"><span>${label}</span><i style="width:${Math.min(100, Math.round(100 * val / max))}%"></i><b>${txt}</b></div>`;
  el.innerHTML = bar('Duration', t.duration_ms, (t.duration_ms / 1000).toFixed(1) + 's') + bar('Model', t.model_ms, (t.model_ms / 1000).toFixed(1) + 's') + bar('Tools', t.tool_ms, (t.tool_ms / 1000).toFixed(1) + 's') + `<div class="stats">Turns ${t.turns} - Calls ${t.calls} - Events ${t.events} - Cost $${(t.cost_micros / 1e6).toFixed(4)}</div>`;
}

async function showPlugins() {
  const el = document.getElementById('plug'), p = await invoke('plugins').catch((e) => ({ tools: [], models: [], error: '' + e }));
  el.innerHTML = '<h3>Tools</h3>' + p.tools.map((t) => `<div class="plg">${t.name}<small>${t.command.split('/').pop()}</small></div>`).join('') + '<h3>Models</h3>' + p.models.map((m) => `<div class="plg">${m.name}${m.default ? ' (default)' : ''}<small>${m.command.split('/').pop()}</small></div>`).join('');
}

async function showSettings() {
  const el = document.getElementById('set'), s = await invoke('settings');
  el.innerHTML = `<h3>Settings</h3><label>Mode <select id="s-mode">${['standard', 'ptc', 'minimal', 'creator'].map((m) => `<option ${m === s.mode ? 'selected' : ''}>${m}</option>`).join('')}</select></label>
    <label>Permission <select id="s-perm">${['ask', 'auto'].map((m) => `<option ${m === s.permission ? 'selected' : ''}>${m}</option>`).join('')}</select></label>
    <label>Max steps <input id="s-steps" type="number" value="${s.max_steps}"></label>
    <label>Project folder <input id="s-proj" placeholder="/path/to/your/project" value="${s.project_dir || ''}"></label><button id="s-save">Save</button><small id="s-msg"></small>`;
  document.getElementById('s-save').onclick = async () => {
    const r = await invoke('save_settings', { patch: { mode: document.getElementById('s-mode').value, permission: document.getElementById('s-perm').value, max_steps: +document.getElementById('s-steps').value, ...(document.getElementById('s-proj').value ? { project_dir: document.getElementById('s-proj').value } : {}) } }).catch((e) => ({ error: '' + e }));
    if (!r.error) await invoke('slash', { line: '/mode ' + r.mode });
    document.getElementById('s-msg').textContent = r.error ? r.error : ' saved';
    if (!r.error) { await invoke('open_engine').catch(() => {}); projectWarn(); }
  };
}

async function pollQuestions() {
  const qs = await invoke('questions').catch(() => []), el = document.getElementById('qcard');
  if (!el) return;
  if (!qs.length) { el.innerHTML = ''; el.style.display = 'none'; return; }
  const q = qs[0]; el.style.display = 'block'; el.innerHTML = '';
  const h = $('<div class="q"></div>'); h.textContent = q.question; el.append(h);
  const done = async (a) => { await invoke('answer', { n: q.n, text: a }); pollQuestions(); };
  if (Array.isArray(q.options)) q.options.forEach((o) => { const b = $('<button></button>'); b.textContent = o; b.onclick = () => done(o); el.append(b); });
  else { const i = $('<input placeholder="Your answer">'); const b = $('<button>Send</button>'); b.onclick = () => done(i.value); el.append(i, b); }
}
setInterval(pollQuestions, 1000);

let pendingFile = '';
async function send(text) {
  text = text.trim(); if (!text) return;
  if (text.startsWith('/')) {
    const r = await invoke('slash', { line: text });
    if (r.attached) { pendingFile += r.attached + '\n'; addDone('attached file for the next goal'); }
    else if (r.queued && !r.queue) addDone('goal started or queued: ' + r.queued);
    else if (r.ok === false) addDone(r.error);
    else addDone(JSON.stringify(r).slice(0, 300));
    return;
  }
  const chat = document.getElementById('chat');
  const ub = $(`<div class="user"></div>`); ub.textContent = text; chat.append(ub);
  const full = pendingFile ? pendingFile + text : text; pendingFile = '';
  const r = await invoke('submit', { goal: full });
  if (r.queued) { addDone('queued behind the running mission'); return; }
  asst = $('<div class="asst"></div>'); chat.append(asst);
  t0 = Date.now(); calls = steps = tin = tout = cached = tinCache = 0; running = {};
  ticker = setInterval(foot, 500);
}

function addDone(t) { const a = asst || document.getElementById('chat'); const d = $(`<div class="done"></div>`); d.textContent = t; a.append(d); }

function foot() {
  const secs = ((Date.now() - t0) / 1000).toFixed(0);
  const live = Object.values(running)[0];
  document.getElementById('f1').textContent = live ? `working ${secs}s - ${live}` : `elapsed ${secs}s`;
  document.getElementById('f2').textContent = `${steps} steps - ${calls} calls`;
  document.getElementById('f3').textContent = `${fmt(tin + tout)} tok - cache hit ${tinCache ? Math.round(100 * cached / tinCache) : 0}%`;
}

listen('hs-event', (e) => {
  const ev = e.payload; traj.push(ev);
  const tj = document.getElementById('traj'); if (false) tj.textContent = `${ev.type} ${ev.plugin || ev.model || ''} ${ev.args || ev.output || ev.text || ''}`.slice(0, 200);
  if (ev.type === 'step') steps = ev.step;
  if (ev.type === 'model_call_end') { calls++; tin += ev.input_tokens; tout += ev.output_tokens; }
  if (ev.type === 'model_call_cache') { cached += ev.cached_tokens; tinCache += ev.input_tokens; }
  if (ev.type === 'reasoning' && asst) { const r = $('<div class="row think"></div>'); r.textContent = 'Thinking...'; const d = $('<div class="d"></div>'); d.textContent = ev.text; r.append(d); r.onclick = () => r.classList.toggle('open'); asst.append(r); }
  if (ev.type === 'tool_start' && asst) { const r = $('<div class="row"></div>'); r.textContent = `${ev.plugin} - running`; const d = $('<div class="d"></div>'); d.textContent = ev.args; r.append(d); r.onclick = () => r.classList.toggle('open'); asst.append(r); running[ev.plugin] = ev.plugin; r._p = ev.plugin; asst._last = r; }
  if (ev.type === 'tool_end' && asst && asst._last) { const r = asst._last; delete running[ev.plugin]; r.firstChild.textContent = `${ev.plugin} - ${ev.ok ? 'Completed' : 'Failed'} in ${(ev.elapsed_ms / 1000).toFixed(1)}s`; r.classList.toggle('fail', !ev.ok); r.querySelector('.d').textContent += '\n=> ' + ev.output; }
});
listen('hs-queue', (e) => { const q = document.getElementById('queue'); if (q) { q.textContent = e.payload.length ? 'Queued: ' + e.payload.join(' | ') : ''; } });
listen('hs-scheduled', (e) => addDone('scheduled run: ' + e.payload.map((x) => x.prompt).join(', ')));
listen('hs-done', (e) => { clearInterval(ticker); const r = e.payload; addDone(`${r.passed ? 'Verified' : 'Not verified'} - ${r.outcome || ''} - ${((Date.now() - t0) / 1000).toFixed(0)}s`); foot(); refresh(); });
boot();


// dsh-style workspaces: registered real folders, an in-app folder browser, click to switch.
async function wsBar(first) {
  const w = await invoke('workspaces'), el = document.getElementById('wsbar');
  if (!el) return;
  el.innerHTML = '<div class="wsh">Workspaces <button id="wsadd">+ Add workspace</button></div>';
  for (const x of w.list) {
    const r = $(`<div class="wsr${x.path === w.active ? ' on' : ''}"><span class="wn"></span><small></small><button class="wsx" title="Remove from list">x</button></div>`);
    r.querySelector('.wn').textContent = x.name; r.querySelector('small').textContent = x.path; r.title = x.path;
    r.onclick = async () => { const e = await invoke('workspace_switch', { path: x.path }).then(() => null).catch((e) => '' + e); if (e) return alert(e); await invoke('open_engine').catch(() => {}); wsBar(); projectWarn(); refresh(); };
    r.querySelector('.wsx').onclick = async (ev) => { ev.stopPropagation(); await invoke('workspace_remove', { path: x.path }); wsBar(); };
    el.append(r);
  }
  document.getElementById('wsadd').onclick = () => browseDialog();
  if (first && !w.active) browseDialog();
}

async function browseDialog(path) {
  const home = path || (await invoke('workspaces').then((w) => w.active || w.home).catch(() => '/')) || '/';
  let b = await invoke('browse', { path: home }).catch(() => invoke('browse', { path: '/' }));
  let dlg = document.getElementById('browse');
  if (!dlg) { dlg = $('<div id="browse"></div>'); document.body.append(dlg); }
  dlg.innerHTML = '<div class="bx"><h3>Choose a project folder</h3><div id="bpath"></div><div id="blist"></div><div class="bbtn"><button id="bup">Up</button><button id="bsel">Use this folder</button><button id="bcan">Cancel</button></div><small id="bmsg"></small></div>';
  dlg.querySelector('#bpath').textContent = b.path;
  const l = dlg.querySelector('#blist');
  for (const d of b.dirs) { const r = $('<div class="bd"></div>'); r.textContent = d + '/'; r.onclick = () => browseDialog(b.path.replace(/\/$/, '') + '/' + d); l.append(r); }
  dlg.querySelector('#bup').onclick = () => b.parent && browseDialog(b.parent);
  dlg.querySelector('#bcan').onclick = () => dlg.remove();
  dlg.querySelector('#bsel').onclick = async () => {
    const w = await invoke('workspace_add', { path: b.path }).catch((e) => ({ error: '' + e }));
    if (w.error) return (dlg.querySelector('#bmsg').textContent = w.error);
    await invoke('workspace_switch', { path: w.path }).catch((e) => alert(e));
    await invoke('open_engine').catch(() => {});
    dlg.remove(); wsBar(); projectWarn(); refresh();
  };
}
