// Real React page with delayed import IPC and out-of-order progress delivery.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const requireFrontend=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=requireFrontend('react'),{create,act}=requireFrontend('react-test-renderer'),ts=requireFrontend('typescript');
const calls=[],listeners=new Map(),timers=[];
const native={recording:false};
let job=null,ack=null,delayAck=false,cancelStatus='requested',failStart=false;
const invoke=async(name,args)=>{
  calls.push({name,args});
  if(name==='interview_status')return {root:'/synthetic',suggested_root:'',aliases:'',facts:'',key_configured:true,index_version:'1',session_active:false};
  if(name==='profile_import_status')return job && {...job};
  if(name==='profile_scan')return [{path:'knowledge.md',bytes:100,eligible:true,reason:'待选择纳入'}];
  if(name==='profile_import'){
    if(failStart)throw Error('synthetic start failure');
    assert.ok(args.importId);assert.equal(args.selected[0].path,'knowledge.md');
    job={import_id:args.importId,phase:'starting',completed:0,total:1,path:null,report:null,message:null};
    if(delayAck)return new Promise(resolve=>{ack=resolve;});return;
  }
  if(name==='profile_import_cancel'){
    assert.equal(args.importId,job.import_id);
    job={...job,phase:cancelStatus==='committing'?'committing':'cancelling'};return cancelStatus;
  }
  if(name==='interview_audio_level')return {system:0,microphone:0,microphone_state:{recording:false,enabled:false,epoch:0}};
  if(name==='answer_cancel')return;
  throw Error('unexpected '+name);
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
    if (name === '@/components/ui/choice-menu') return {ChoiceMenu:'choice-menu',ChoiceOption:'choice-option'};
    if (name === '@/components/ui/button') return {Button:props=>React.createElement('button',props)};
    if (name === '@/components/ui/input') return {Input:props=>React.createElement('input',props)};
    if (name === 'next/link') return props=>React.createElement('a',props);
    if (name === 'lucide-react') return new Proxy({}, {get:()=>()=>null});
    if (name.startsWith('@/components/interview/')) return load(name.slice(2)+'.tsx');
    if (name.startsWith('@/lib/')) return load(name.slice(2)+'.ts');
    if (name === './interview-scheduler') return load('lib/interview-scheduler.ts');
    return requireFrontend(name);
  };
  new Function('require','module','exports','setInterval','clearInterval',code)(requireMock,module,module.exports,(callback,delay)=>{timers.push({callback,delay});return timers.length;},()=>{});
  return cache[absolute] = module.exports;
}
function text(node) { return typeof node === 'string' ? node : (node?.children||[]).map(text).join(''); }
(async()=>{
  let tree;
  const mount=async()=>{await act(async()=>{tree=create(React.createElement(load('app/interview/page.tsx').default));});};
  const button=label=>tree.root.findAllByType('button').find(node=>text(node)===label);
  const click=async label=>{const b=button(label);assert.ok(b,label);assert.equal(!!b.props.disabled,false,label+' disabled');await act(async()=>{await b.props.onClick();});};
  const progress=async (phase, extra={})=>{job={...job,phase,...extra};await act(async()=>{listeners.get('profile-import')({payload:{...job}});});};
  await mount();
  await click('资料与设置');await click('扫描预览');
  const include=tree.root.findByProps({'aria-label':'纳入 knowledge.md'});
  await act(async()=>{include.props.onChange({target:{checked:true}});});
  delayAck=true;
  await act(async()=>{void button('导入 / 更新所选资料').props.onClick();});
  assert.equal(button('取消导入').props.disabled,true,'cancel before native job exists');
  await progress('parsing',{path:'knowledge.md'});
  assert.equal(button('取消导入').props.disabled,false,'generic busy flag must not disable cancellation');
  assert.equal(button('导入 / 更新所选资料').props.disabled,true);
  const oldId=job.import_id;
  await click('取消导入');assert.ok(text(tree.toJSON()).includes('正在取消导入'));
  assert.equal(button('取消导入').props.disabled,true);
  await act(async()=>{ack();});
  await progress('cancelled',{message:'导入已取消，原资料库保持不变'});
  assert.equal(button('取消导入'),undefined);assert.equal(button('导入 / 更新所选资料').props.disabled,false);
  assert.ok(text(tree.toJSON()).includes('原资料库保持不变'));
  delayAck=false;await click('导入 / 更新所选资料');assert.notEqual(job.import_id,oldId);
  await progress('parsing');
  await act(async()=>{listeners.get('profile-import')({payload:{...job,import_id:oldId,phase:'failed',message:'迟到的旧错误'}});});
  assert.ok(!text(tree.toJSON()).includes('迟到的旧错误'));assert.ok(button('取消导入'));
  cancelStatus='committing';await click('取消导入');
  assert.ok(text(tree.toJSON()).includes('正在完成保存'));assert.equal(button('取消导入').props.disabled,true);
  job={...job,phase:'completed',report:{version:2,success:1},path:null};
  await act(async()=>{timers.find(t=>t.delay===500).callback();});
  assert.equal(button('取消导入'),undefined);assert.ok(text(tree.toJSON()).includes('"version": 2'),'poll must recover missed terminal event');
  failStart=true;await click('导入 / 更新所选资料');
  assert.ok(text(tree.toJSON()).includes('synthetic start failure'));assert.equal(button('导入 / 更新所选资料').props.disabled,false);
  failStart=false;cancelStatus='requested';await click('导入 / 更新所选资料');await progress('parsing');
  await act(async()=>{tree.unmount();});await mount();
  assert.ok(button('取消导入'),'active import must be recoverable after leaving/reopening the page, even with settings collapsed');
  await click('取消导入');await progress('cancelled',{message:'导入已取消，原资料库保持不变'});
  assert.equal(calls.filter(c=>c.name==='interview_audio_start'||c.name==='answer_start').length,0,'import must never start capture or AI');
  await act(async()=>tree.unmount());
  console.log('PASS: real page import cancellation stays usable while busy; pre-ack/progress race, rollback message, retry, stale events, commit race, missed terminal polling, failure reset, remount recovery, no capture/API');
})().catch(error=>{console.error(error);process.exitCode=1;});
