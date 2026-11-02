// Exercises the real page's controls with a strict native command contract.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const requireFrontend = require('node:module').createRequire(path.resolve(__dirname, '../frontend/package.json'));
const React = requireFrontend('react');
const { create, act } = requireFrontend('react-test-renderer');
const ts = requireFrontend('typescript');
const calls = [], listeners = new Map();
let native = { recording: false, enabled: false, epoch: 0 }, failClear = false;
const invoke = async (name, args) => {
  calls.push({ name, args });
  if (name === 'interview_status') return { root:'', suggested_root:'', aliases:'', facts:'', key_configured:true, index_version:'1' };
  if (name === 'interview_audio_start') {
    assert.equal(typeof args?.recordMicrophone, 'boolean');
    assert.equal(native.recording, false, 'start while native capture still active');
    native = { recording:true, enabled:args.recordMicrophone, epoch:native.epoch+1 }; return {...native};
  }
  if (name === 'stop_recording') {
    assert.equal(typeof args?.args?.save_path, 'string', 'native command requires args.save_path');
    native.recording = false; native.enabled = false; return;
  }
  if (name === 'get_recording_state') return { is_recording:native.recording };
  if (name === 'interview_microphone_set') {
    assert.equal(native.recording, true); native.enabled = args.enabled; native.epoch++; return {...native};
  }
  if (name === 'interview_audio_level') return {system:0,microphone:0,microphone_state:{...native}};
  if (name === 'interview_history_clear') {
    assert.equal(native.recording, false, 'clear before stopping capture');
    if (failClear) throw Error('synthetic Trash failure');
    return {archive_path:'/synthetic/Trash/archive',recording_folders:1,database_meetings:1};
  }
  if (['answer_cancel','session_clear'].includes(name)) return;
  throw Error('unexpected command: '+name);
};
const cache = {};
function load(relative) {
  const absolute = path.resolve(__dirname, '../frontend/src', relative);
  if (cache[absolute]) return cache[absolute];
  const code = ts.transpileModule(fs.readFileSync(absolute,'utf8'), {compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX,esModuleInterop:true}}).outputText;
  const module = {exports:{}};
  const requireMock = name => {
    if (name === '@tauri-apps/api/core') return { invoke, isTauri:()=>true };
    if (name === '@tauri-apps/api/event') return {listen:async (event,callback)=>{listeners.set(event,callback);return ()=>listeners.delete(event);}};
    if (name === '@/contexts/RecordingStateContext') return {useRecordingState:()=>({isRecording:native.recording,isActive:native.recording,isPaused:false,isStopping:false})};
    if (name === '@/contexts/TranscriptContext') return {useTranscripts:()=>({resetAfterHistoryClear:()=>{}})};
    if (name === '@/services/indexedDBService') return {indexedDBService:{suspendRecordingWrites(){},snapshotInterviewHistory:async()=>({meetings:[],transcripts:[]}),removeArchivedInterviewHistory:async()=>{}}};
    if (name === '@/components/ui/button') return {Button:props=>React.createElement('button',props)};
    if (name === '@/components/ui/input') return {Input:props=>React.createElement('input',props)};
    if (name === 'next/link') return props=>React.createElement('a',props);
    if (name === 'lucide-react') return new Proxy({}, {get:()=>()=>null});
    if (name.startsWith('@/lib/')) return load(name.slice(2)+'.ts');
    if (name === './interview-scheduler') return load('lib/interview-scheduler.ts');
    return requireFrontend(name);
  };
  new Function('require','module','exports','setInterval','clearInterval',code)(requireMock,module,module.exports,()=>0,()=>{});
  return cache[absolute] = module.exports;
}
function text(node) { return typeof node === 'string' ? node : (node?.children||[]).map(text).join(''); }
(async()=>{
  let tree;
  await act(async()=>{tree=create(React.createElement(load('app/interview/page.tsx').default));});
  const button = label => tree.root.findAllByType('button').find(node=>text(node)===label);
  const click = async label => {const b=button(label);assert.ok(b, label);assert.equal(!!b.props.disabled,false,label+' disabled');await act(async()=>{await b.props.onClick();});};
  const check = async (label,value) => {
    const node = tree.root.findAllByType('label').find(node=>text(node).includes(label));assert.ok(node,label);
    await act(async()=>{await node.findByType('input').props.onChange({target:{checked:value}});});
  };
  const emit = async payload => {await act(async()=>{listeners.get('transcript-update')({payload});});};
  await check('已获允许使用转写',true);await check('允许向 DeepSeek',true);
  await check('记录我的声音',false);
  await click('开始持续聆听与回答');assert.equal(native.enabled,false);
  await check('记录我的声音',true);assert.equal(native.enabled,true);
  const firstEpoch=native.epoch;
  const own=(id,epoch,words)=>({sequence_id:id,source:'Microphone',microphone_epoch:epoch,text:words,is_partial:false,timestamp:'00:01',audio_start_time:0,audio_end_time:1});
  await emit(own(1,firstEpoch,'自己的答案保留'));assert.ok(text(tree.toJSON()).includes('自己的答案保留'));
  await check('记录我的声音',false);
  await emit(own(2,firstEpoch,'关闭后的旧转写'));assert.ok(!text(tree.toJSON()).includes('关闭后的旧转写'));
  await check('记录我的声音',true);
  await emit(own(3,firstEpoch,'重开后迟到的旧转写'));assert.ok(!text(tree.toJSON()).includes('重开后迟到的旧转写'));
  await emit(own(4,native.epoch,'重开后的新回答'));assert.ok(text(tree.toJSON()).includes('重开后的新回答'));
  assert.equal(calls.filter(c=>c.name==='answer_start').length,0);
  await click('停止采集');assert.equal(native.recording,false);
  await click('开始持续聆听与回答');assert.equal(native.recording,true);
  await click('结束会话并保留历史');assert.equal(native.recording,false);
  assert.ok(text(tree.toJSON()).includes('自己的答案保留'));
  await click('开始持续聆听与回答');
  failClear=true;await click('清空历史，重新开始');assert.equal(native.recording,false);
  assert.ok(text(tree.toJSON()).includes('自己的答案保留'),'failed Trash operation erased history');
  failClear=false;await click('开始持续聆听与回答');await click('清空历史，重新开始');
  assert.equal(native.recording,false);assert.ok(!text(tree.toJSON()).includes('自己的答案保留'));
  await click('开始持续聆听与回答');assert.equal(native.recording,true);
  assert.equal(calls.filter(c=>c.name==='stop_recording').length,4);
  await act(async()=>tree.unmount());
  console.log('PASS: actual Stop/End/Clear controls pass native args; stop/start and clear/start work without remount; failed Trash keeps history');
  console.log('PASS: start with mic off, live off/on, stale epoch delivery blocked, new self speech retained, no self AI request');
})().catch(error=>{console.error(error);process.exitCode=1;});
