// Serve the exported display window with synthetic Tauri presentation events.
const fs = require('node:fs');
const path = require('node:path');
const output = process.argv[2];
if (!output) throw Error('Pass a new output directory');
const build = path.resolve(__dirname, '../frontend/out');
const page = fs.readFileSync(path.join(build, 'interview-display.html'), 'utf8');

function fixture() {
  const paragraph = '先按租户验证幂等键，再分别验证字段缺失、null 和零。';
  let sequence = 0, version = 0, epoch = 'synthetic', focus = false;
  const callbacks = new Map();
  let listener;
  const row = (n, answered) => ({
    id: 'q' + n, role: 'remote', time: '10:00', question: '第 ' + n + ' 题：怎样验证接口？',
    status: answered ? 'done' : 'listening', suggestion: answered ? '答复 ' + n + '。' + paragraph.repeat(5) : ''
  });
  let history = Array.from({length:18}, (_,i) => row(i+1, true));
  const snapshot = () => ({epoch,version:++version,history:history.slice(),selected_id:null,preferences:{focus,showSelf:true,fontSize:'medium'}});
  const send = () => listener?.({event:'interview-presentation',id:1,payload:snapshot()});
  window.__TAURI_INTERNALS__ = {
    transformCallback(fn) { callbacks.set(++sequence,fn); return sequence; },
    unregisterCallback(id) { callbacks.delete(id); },
    invoke: async (cmd,args) => {
      if (cmd === 'interview_presentation_get') return snapshot();
      if (cmd === 'plugin:event|listen') { listener = callbacks.get(args.handler); return 1; }
      if (cmd === 'plugin:event|unlisten') return;
      throw Error('Unexpected bridge command: ' + cmd);
    }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener() {}};
  window.addEventListener('load', () => {
    const bar = document.createElement('aside');
    bar.setAttribute('aria-label','合成测试输入');
    bar.style.cssText = 'position:fixed;bottom:0;left:0;right:0;height:46px;background:white;display:flex;align-items:center;justify-content:center;gap:20px;z-index:10';
    for (const [label, action] of [
      ['新增转写', () => history.push(row(history.length+1, false))],
      ['更新转写', () => history[history.length-1]={...history.at(-1),question:history.at(-1).question+paragraph.repeat(10)}],
      ['生成答复', () => history[history.length-1]={...history.at(-1),status:'generating',suggestion:'新答复。'+(paragraph+'\n').repeat(32)}],
      ['续写答复', () => history[history.length-1]={...history.at(-1),suggestion:history.at(-1).suggestion+(paragraph+'\n').repeat(20)}],
      ['纠正问句', () => history[history.length-1]={...history.at(-1),status:'done',suggestion:history.at(-1).suggestion+'\n<<<QUESTION>>>\n'+paragraph.repeat(25)}],
      ['单题或列表', () => { focus=!focus; }],
      ['清空记录', () => { history=[];epoch+='-new'; }]
    ]) {
      const btn=document.createElement('button'); btn.textContent=label;
      btn.onclick=()=>{action();send();};bar.appendChild(btn);
    }
    document.body.appendChild(bar);
  });
}
fs.mkdirSync(output);
fs.symlinkSync(path.join(build,'_next'), path.join(output,'_next'), 'dir');
const html=page.replace('<head>', '<head><style>.interview-display{height:calc(100dvh - 48px)!important}</style><script>('+fixture.toString()+')()</script>');
fs.writeFileSync(path.join(output,'interview-display.html'),html);
console.log(path.join(output,'interview-display.html'));
