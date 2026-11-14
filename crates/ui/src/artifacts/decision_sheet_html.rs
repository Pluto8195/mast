//! Interactive controls for the decision-sheet frame.
use mast_proto::decision_sheet::{SheetDecision, SheetDecisionDrafts};

pub(crate) fn with_state(
    source: &str,
    drafts: Option<&SheetDecisionDrafts>,
    sent: &[SheetDecision],
) -> String {
    let state = serde_json::json!({"drafts": drafts.map(|d| &d.decisions), "sent": sent})
        .to_string()
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    format!("{source}<style>{CSS}</style><script>window.__mastSheetState={state};{SCRIPT}</script>")
}

const CSS: &str = r#"
.mk-sheet .mk-decision { display:grid; grid-template-columns:minmax(0,1fr) auto; gap:6px 24px; padding:14px 0; }
.mk-sheet .mk-decision > div:first-child { min-width:0; }
.mk-sheet-actions { display:flex; flex-wrap:wrap; align-items:center; gap:8px; margin-top:8px; }
.mk-sheet-actions[hidden], .mk-sheet-note[hidden] { display:none; }
.mk-sheet-actions button, .mk-sheet-bar button { font:inherit; font-size:13px; font-weight:600; border:0; border-radius:14px; padding:4px 14px; cursor:pointer; color:var(--foreground); }
.mk-sheet-actions [data-sheet-accept], .mk-sheet-actions [data-sheet-option], .mk-sheet-bar [data-sheet-send] { background:color-mix(in srgb,var(--mk-green) 30%,var(--background)); }
.mk-sheet-actions [data-sheet-reject], .mk-sheet-actions [data-sheet-answer] { background:color-mix(in srgb,var(--foreground) 6%,var(--background)); color:var(--muted-foreground); }
.mk-sheet-actions [data-sheet-change], .mk-sheet-actions [data-sheet-undo], .mk-sheet-actions [data-sheet-note-toggle], .mk-sheet-bar [data-sheet-discard] { background:transparent; color:var(--muted-foreground); padding:4px 3px; }
.mk-sheet .mk-sheet-note { grid-column:1 / -1; color:var(--muted-foreground); font-size:12px; }
.mk-sheet-note input { box-sizing:border-box; display:block; width:100%; margin-top:4px; padding:6px 8px; font:inherit; color:var(--foreground); background:var(--background); border:1px solid var(--border); border-radius:6px; }
.mk-sheet-staged { white-space:nowrap; border:0; text-decoration:none; }
.mk-sheet-accepted { color:var(--foreground); background:color-mix(in srgb,var(--mk-green) 22%,var(--background)); }
.mk-sheet-rejected { color:var(--muted-foreground); background:var(--muted); }
.mk-sheet-bar { position:sticky; top:0; display:flex; flex-wrap:wrap; align-items:center; gap:10px; padding:10px 0; background:var(--background); z-index:2; }
.mk-sheet-bar[hidden] { display:none; }
@container (max-width:560px) { .mk-sheet .mk-decision { gap:6px 12px; } }
"#;

const SCRIPT: &str = r#"(() => {
  const initial = window.__mastSheetState || {};
  const drafts = new Map((initial.drafts || []).map(d => [d.rowId,d]));
  const sent = new Map((initial.sent || []).map(d => [d.rowId,d]));
  const changing = new Set(), noteOpen = new Set(), answering = new Set(), errors = new Map();
  const notes = new Map((initial.drafts || []).map(d => [d.rowId,d.note || '']));
  const post = o => { try { parent.postMessage({mastMsg:o}, '*'); } catch (_) {} };
  const bar = document.createElement('div'); bar.className = 'mk-sheet-bar';
  const label = document.createElement('span');
  const send = document.createElement('button'); send.type='button'; send.textContent='Send'; send.setAttribute('data-sheet-send','');
  const discard = document.createElement('button'); discard.type='button'; discard.textContent='Discard'; discard.setAttribute('data-sheet-discard','');
  bar.append(label, send, discard); const sheet = document.querySelector('.mk-sheet'); sheet.insertBefore(bar, sheet.querySelector('h1').nextSibling);
  document.querySelectorAll('[data-sheet-row]').forEach(row => {
    const controls = row.querySelector('.mk-sheet-actions');
    if (!controls) {
      const saved = sent.get(row.getAttribute('data-sheet-row'));
      if (saved && saved.outcome === 'option') { const chosen = document.createElement('div'); chosen.className='mk-recommendation'; chosen.textContent=`Chosen: ${saved.label}`; row.firstElementChild.appendChild(chosen); }
      return;
    }
    const note = controls.querySelector('label'); const input = note.querySelector('input');
    note.className = 'mk-sheet-note'; note.textContent = 'Note'; note.appendChild(input); note.hidden = true; row.appendChild(note);
    const toggle = document.createElement('button'); toggle.type='button'; toggle.textContent='Note'; toggle.setAttribute('data-sheet-note-toggle',''); toggle.setAttribute('aria-label','Add note');
    const change = document.createElement('button'); change.type='button'; change.textContent='Change'; change.setAttribute('data-sheet-change','');
    controls.insertBefore(change, controls.querySelector('[data-sheet-undo]')); controls.appendChild(toggle);
  });
  const update = () => {
    bar.hidden = drafts.size === 0; label.textContent = `${drafts.size} answers ready`; send.disabled = errors.size > 0;
    document.querySelectorAll('[data-sheet-row]').forEach(row => {
      const id = row.getAttribute('data-sheet-row'); const controls = row.querySelector('.mk-sheet-actions');
      if (!controls) return;
      const saved = sent.get(id), draft = drafts.get(id), editing = changing.has(id);
      controls.hidden = !!saved;
      const chip = row.querySelector(':scope > .mk-chip');
      if (chip) {
        const result = saved || draft;
        chip.className = result ? `mk-chip mk-sheet-staged ${result.outcome !== 'rejected' ? 'mk-sheet-accepted' : 'mk-sheet-rejected'}` : 'mk-chip mk-open';
        chip.textContent = result ? `${result.outcome === 'rejected' ? "Won't do" : result.outcome === 'option' ? `Chose: ${result.label}` : result.overrideRecommendation ? 'Answered' : 'Accepted'}${draft && !saved ? ' · not sent' : ''}` : 'Open';
      }
      const accept = controls.querySelector('[data-sheet-accept]'); if (accept) accept.hidden = !!draft && !editing;
      controls.querySelectorAll('[data-sheet-option]').forEach(button => button.hidden = !!draft && !editing);
      const reject = controls.querySelector('[data-sheet-reject]'); if (reject) reject.hidden = !!draft && !editing;
      controls.querySelector('[data-sheet-answer]').hidden = !!draft && !editing;
      controls.querySelector('[data-sheet-answer]').textContent = answering.has(id) ? 'Stage answer' : 'Answer differently';
      controls.querySelector('[data-sheet-change]').hidden = !draft || editing;
      controls.querySelector('[data-sheet-undo]').hidden = !draft;
      const status = controls.querySelector('[data-sheet-status]');
      status.textContent = errors.has(id) ? `Not staged: ${errors.get(id)}` : ''; status.hidden = !errors.has(id);
      const note = row.querySelector('.mk-sheet-note'); const input = note.querySelector('input');
      note.hidden = !!saved || !noteOpen.has(id);
      note.firstChild.textContent = answering.has(id) ? 'Your answer' : 'Note';
      input.placeholder = answering.has(id) ? 'Your answer' : 'Note';
      if (document.activeElement !== input) input.value = notes.get(id) || '';
    });
  };
  document.querySelectorAll('[data-sheet-row]').forEach(row => {
    const id = row.getAttribute('data-sheet-row'), controls = row.querySelector('.mk-sheet-actions');
    if (!controls || sent.has(id)) return;
    const input = row.querySelector('.mk-sheet-note input');
    const stage = (outcome, overrideRecommendation=false, optionId=null, optionLabel=null) => { errors.delete(id); const note=input.value; if (overrideRecommendation && !note.trim()) { errors.set(id,'Your answer is required.'); update(); input.focus(); return; } notes.set(id,note); drafts.set(id,{rowId:id,outcome,note,overrideRecommendation,optionId,label:optionLabel}); answering.delete(id); changing.delete(id); post({t:'sheet-stage',id,outcome,note,overrideRecommendation,optionId}); update(); };
    const accept = controls.querySelector('[data-sheet-accept]'); if (accept) accept.onclick = () => stage('decided');
    controls.querySelectorAll('[data-sheet-option]').forEach(button => button.onclick = () => stage('option',false,button.getAttribute('data-sheet-option'),button.dataset.label));
    const reject = controls.querySelector('[data-sheet-reject]'); if (reject) reject.onclick = () => stage('rejected');
    controls.querySelector('[data-sheet-answer]').onclick = () => { if (answering.has(id)) stage('decided',true); else { answering.add(id); noteOpen.add(id); update(); input.focus(); } };
    controls.querySelector('[data-sheet-change]').onclick = () => { changing.add(id); update(); };
    controls.querySelector('[data-sheet-undo]').onclick = () => { drafts.delete(id); notes.delete(id); noteOpen.delete(id); changing.delete(id); answering.delete(id); errors.delete(id); input.value=''; post({t:'sheet-undo',id}); update(); };
    controls.querySelector('[data-sheet-note-toggle]').onclick = () => { if (noteOpen.has(id)) noteOpen.delete(id); else noteOpen.add(id); update(); if (noteOpen.has(id)) input.focus(); };
    input.oninput = () => { notes.set(id,input.value); const draft = drafts.get(id); if (draft && !changing.has(id) && !answering.has(id)) { if (draft.overrideRecommendation && !input.value.trim()) { errors.set(id,'Your answer is required.'); } else { errors.delete(id); draft.note=input.value; post({t:'sheet-stage',id,outcome:draft.outcome,note:input.value,overrideRecommendation:!!draft.overrideRecommendation,optionId:draft.optionId}); } update(); } };
  });
  discard.onclick = () => { drafts.clear(); notes.clear(); noteOpen.clear(); changing.clear(); answering.clear(); errors.clear(); post({t:'sheet-discard'}); update(); };
  const setBusy = busy => { send.disabled=busy; discard.disabled=busy; document.querySelectorAll('.mk-sheet-actions button,.mk-sheet-note input').forEach(node => node.disabled=busy); };
  send.onclick = () => { setBusy(true); post({t:'sheet-send'}); };
  window.addEventListener('message', e => { if (!e.data) return; if (e.data.mastSheetStageRejected) { const {id,reason} = e.data.mastSheetStageRejected; drafts.delete(id); errors.set(id,reason); setBusy(false); update(); } if (e.data.mastSheetEnableSend) setBusy(false); if (e.data.mastSheetSent) { e.data.mastSheetSent.forEach(d => {sent.set(d.rowId,d); drafts.delete(d.rowId);}); setBusy(false); update(); } });
  update();
})();"#;
