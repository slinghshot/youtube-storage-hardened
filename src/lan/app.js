'use strict';
const el = id => document.getElementById(id);
let mode = 'create', uploading = false;
const message = text => { el('message').textContent = text; };
function signedIn(ok) {
  el('workspace').hidden = el('logout').hidden = !ok;
  el('loginBox').hidden = ok;
}
function selectMode(next) {
  if (uploading) return;
  mode = next;
  const create = mode === 'create';
  for (const id of ['audioLabel', 'typeLabel', 'confirmLabel']) el(id).hidden = !create;
  for (const id of ['create','restore']) el(id).setAttribute('aria-pressed', String(id === mode));
  el('source').webkitdirectory = create && el('type').value === 'Folder';
  el('source').value = el('audio').value = el('confirm').value = el('pass').value = '';
  el('source').accept = create ? '' : 'video/*';
  el('sourceLabel').textContent = create ? 'Choose a ' + el('type').value.toLowerCase() : 'Backup MP4';
  el('title').textContent = create ? 'Create backup' : 'Restore backup';
  el('submit').textContent = create ? 'Upload and create' : 'Upload and restore';
  message('');
}
el('create').onclick = () => selectMode('create');
el('restore').onclick = () => selectMode('restore');
el('type').onchange = () => selectMode(mode);
el('login').onsubmit = async event => {
  event.preventDefault();
  try {
    const response = await fetch('/login',{method:'POST',body:new URLSearchParams(new FormData(event.target))});
    event.target.reset();
    if (!response.ok) throw Error(response.status === 429 ? 'Too many attempts. Try again in a minute.' : 'Sign-in failed.');
    message(''); await refresh();
  } catch(e) { message(e.message); }
};
el('logout').onclick = async () => {
  if ((await fetch('/logout',{method:'POST'})).ok) { selectMode('create'); signedIn(false); }
};
el('upload').onsubmit = event => {
  event.preventDefault();
  if (uploading) return;
  if (mode === 'create' && el('pass').value !== el('confirm').value) return message('Passphrases differ.');
  const form = new FormData();
  form.append('password',el('pass').value); form.append('mode',mode);
  for (const file of el('source').files) form.append('file:'+(file.webkitRelativePath || file.name),file,'upload');
  if (mode === 'create' && el('audio').files.length) form.append('audio',el('audio').files[0],'audio');
  const xhr = new XMLHttpRequest();
  xhr.open('POST','/jobs');
  uploading = true; el('submit').disabled = el('logout').disabled = true;
  el('progress').hidden = false; el('progress').value = 0;
  message('Uploading. Keep this page open until the server accepts the job.');
  xhr.upload.onprogress = e => { if (e.lengthComputable) el('progress').value = 100*e.loaded/e.total; };
  xhr.onloadend = () => {
    uploading = false; el('submit').disabled = el('logout').disabled = false; el('progress').hidden = true;
    message(xhr.status === 202 ? 'Upload accepted. You can close this page.' :
      ({409:'Another upload or conversion is running.',413:'Upload exceeds the configured limit.',507:'Insufficient storage.',401:'Session expired.'}[xhr.status] || 'Upload failed. Check file selection and server availability.'));
    refresh();
  };
  xhr.send(form); el('pass').value = el('confirm').value = '';
};
async function refresh() {
  try {
    const res = await fetch('/jobs');
    if (res.status === 401) { signedIn(false); return; }
    if (!res.ok) throw Error('Could not load jobs.');
    const rows = await res.json(); signedIn(true);
    el('jobs').replaceChildren();
    if (!rows.length) el('jobs').textContent = 'No jobs yet.';
    for (const row of rows) {
      const box = document.createElement('div'); box.className = 'job';
      const title = document.createElement('strong');
      title.textContent = row.status + ' · ' + row.id.substring(0,8); box.append(title);
      if (row.expires) {
        const time = document.createElement('div'); time.className='muted';
        time.textContent = 'Expires '+new Date(row.expires*1000).toLocaleString(); box.append(time);
        if(row.status==='Ready'){
          const get = document.createElement('button'); get.textContent='Download';
          get.onclick=()=>{el('download').action='/jobs/'+row.id+'/download'; el('unlock').showModal();}; box.append(get);
        }
        const del = document.createElement('button'); del.textContent='Delete'; del.className='secondary';
        del.onclick=async()=>{if(window.confirm('Delete this job and its files?')){
          const r=await fetch('/jobs/'+row.id+'/delete',{method:'POST'});
          if(!r.ok) message('Delete failed. An active download may be using this job.'); await refresh();
        }};box.append(del);
      }
      el('jobs').append(box);
    }
  } catch(e) { message(e.message); }
}
el('cancel').onclick=()=>el('unlock').close();
el('unlock').onclose=()=>el('download').reset();
el('download').onsubmit=()=>setTimeout(()=>el('unlock').close(),0);
refresh(); setInterval(()=>{if(!el('workspace').hidden) refresh();},3000);
