// Real basic/workspace page controls, isolated native transport and panel stubs.
// No audio device, user database or provider is accessed.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=req('react'),{create,act}=req('react-test-renderer'),ts=req('typescript');
const text=n=>typeof n==='string'?n:(n?.children||[]).map(text).join('');
async function exercise(page){
  let tree,recording=false,paused=false,holdResume=false,releaseResume,failResume=false,failPause=false;
  const calls=[],listeners=new Map(),timers=[],cache={};
  let health={session_id:37,recording:true,active:true,paused:false,system_started:true,microphone_enabled:false,active_for_ms:10000,elapsed_ms:1,system_frame_age_ms:0,microphone_frame_age_ms:null,system_signal_age_ms:null,asr_pending:0,asr_no_progress_ms:null,fault:null};
  const native=()=>({recording,enabled:false,epoch:1,health:recording?health:null});
  const invoke=async(name,args)=>{
    calls.push({name,args});
    if(name.startsWith('interview_memory_'))return require('./test-support/memory-disabled.cjs')(name);
    if(name==='interview_status')return {root:'',suggested_root:'',aliases:'',facts:'',documents:[],index_documents:0,index_chunks:0,index_version:'0',key_configured:page==='interview',workspace_ready:true,answer_capacity:2,cloud_asr_mode:'aliyun',session_active:false,preferences:{scenario:'interview'}};
    if(name==='profile_import_status')return null;
    if(name==='interview_window_mode_get')return false;
    if(name==='interview_audio_start'){recording=true;return native();}
    if(name==='interview_audio_level')return {system:0,microphone:0,microphone_state:native()};
    if(name==='answer_cancel')return;
    if(name==='pause_recording'){if(failPause)throw Error('synthetic pause error');paused=true;return;}
    if(name==='resume_recording'){if(holdResume)await new Promise(resolve=>releaseResume=resolve);if(failResume)throw Error('synthetic resume error');paused=false;return;}
    if(name==='stop_recording'){recording=false;paused=false;return;}
    if(name==='get_recording_state')return {is_recording:recording};
    throw Error('Unexpected IPC: '+name);
  };
  function load(relative){
    const absolute=path.resolve(__dirname,'../frontend/src',relative);
    if(cache[absolute])return cache[absolute];
    const code=ts.transpileModule(fs.readFileSync(absolute,'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX,esModuleInterop:true}}).outputText;
    const mod={exports:{}};
    const requireMock=name=>{
      if(name==='@tauri-apps/api/core')return {invoke,isTauri:()=>true};
      if(name==='@tauri-apps/api/event')return {listen:async(event,cb)=>{listeners.set(event,cb);return()=>listeners.delete(event);}};
      if(name==='@/contexts/RecordingStateContext')return {useRecordingState:()=>({isRecording:recording,isActive:recording&&!paused,isPaused:paused,isStopping:false})};
      if(name==='@/contexts/TranscriptContext')return {useTranscripts:()=>({resetAfterHistoryClear(){}})};
      if(name==='@/services/indexedDBService')return {indexedDBService:{}};
      if(name==='@/lib/use-interview-recovery')return {useInterviewRecovery:()=>({drafts:[],error:null,writer:{suspend(){},resume(){},flush:async()=>{},retry:async()=>{}}})};
      if(name==='@/components/interview/MeetingMode')return load('components/interview/MeetingMode.tsx');
      if(name.startsWith('@/components/interview/'))return new Proxy({},{get:()=>()=>null});
      if(name==='@/components/ui/choice-menu')return {ChoiceMenu:'choice-menu',ChoiceOption:'choice-option'};
      if(name==='@/components/ui/button')return {Button:p=>React.createElement('button',p)};
      if(name==='@/components/ui/input')return {Input:p=>React.createElement('input',p)};
      if(name==='next/link')return p=>React.createElement('a',p);
      if(name==='lucide-react')return new Proxy({},{get:()=>()=>null});
      if(name.startsWith('@/lib/'))return load(name.slice(2)+'.ts');
      if(name.startsWith('./'))return load(path.relative(path.resolve(__dirname,'../frontend/src'),path.resolve(path.dirname(absolute),name+'.ts')));
      return req(name);
    };
    new Function('require','module','exports','setInterval','clearInterval',code)(requireMock,mod,mod.exports,(callback,delay)=>{const t={callback,delay};timers.push(t);return t;},t=>{const i=timers.indexOf(t);if(i>=0)timers.splice(i,1);});
    return cache[absolute]=mod.exports;
  }
  await act(async()=>{tree=create(React.createElement(load(`app/${page}/page.tsx`).default));});
  const button=label=>tree.root.findAllByType('button').find(n=>text(n)===label);
  const click=async label=>{const b=button(label);assert.ok(b,label);assert.ok(!b.props.disabled,label);await act(async()=>b.props.onClick());};
  const checkbox=label=>tree.root.findAllByType('label').find(n=>text(n).includes(label)).findByType('input');
  const check=async(label,value)=>{await act(async()=>checkbox(label).props.onChange({target:{checked:value}}));};
  const automatic=()=>checkbox('对方说完后自动回答').props.checked;
  const phase=()=>tree.root.findAllByProps({role:'status'}).map(n=>n.props['aria-label']||text(n)).join(' ');
  const share=page==='interview'?'允许向 DeepSeek':'允许向已启用的 AI';
  await check('已获允许使用转写',true);await check('允许将本次声音',true);await check(share,true);
  await click(page==='interview'?'开始持续聆听与回答':'开始持续聆听');
  assert.equal(automatic(),true,'ready alternate provider should work in workspace');
  await check('对方说完后自动回答',false);
  await click('暂停');assert.equal(phase(),'采集已暂停');
  await click('恢复');assert.equal(automatic(),false,'resume must preserve manual AI off');
  assert.match(phase(),/监听中.*百炼实时转写/);assert.doesNotMatch(phase(),/取消/);
  await check('对方说完后自动回答',true);await click('暂停');await click('恢复');
  assert.equal(automatic(),true);assert.match(phase(),/对方说完后回答/);
  // Revocation while awaiting native resume must beat the captured render's consent.
  await click('暂停');holdResume=true;
  let pending;await act(async()=>{pending=button('恢复').props.onClick();});
  await check(share,false);holdResume=false;await act(async()=>{releaseResume();await pending;});
  assert.equal(automatic(),false,'late resume ignored revoked sharing consent');
  // Failed native operations never claim success or enable automatic sends.
  failPause=true;await click('暂停');assert.match(phase(),/暂停未确认/);assert.equal(automatic(),false);
  failPause=false;await click('暂停');failResume=true;await click('恢复');
  assert.match(phase(),/恢复未确认/);assert.equal(automatic(),false);
  failResume=false;await click('恢复');assert.equal(automatic(),false);
  // Local-only recovery remains available, and does not opt back into AI.
  const fault={session_id:37,sequence:1,code:'capture_error'};
  health={...health,elapsed_ms:2,system_frame_age_ms:null,fault};
  await act(async()=>listeners.get('interview-audio-fault')({payload:fault}));
  health={...health,elapsed_ms:3,system_frame_age_ms:0};
  await act(async()=>timers.find(t=>t.delay===500).callback());
  await click('确认声音恢复，继续转写');assert.equal(automatic(),false);assert.match(phase(),/百炼实时转写/);
  assert.equal(calls.filter(c=>/answer_start|connection_test/.test(c.name)).length,0);
  await click('停止采集');await act(async()=>tree.unmount());
  console.log(`PASS ${page}: pause/resume state, manual off, current consent, native failure, local recovery, no model calls`);
}
(async()=>{await exercise('interview');await exercise('interview-workspace');})().catch(e=>{console.error(e);process.exitCode=1;});
