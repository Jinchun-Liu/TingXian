// Test the actual exit controller with deferred IPC and failure at each step.
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const fromFrontend=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const ts=fromFrontend('typescript');
const mod={exports:{}};
const compiled=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/lib/interview-exit.ts'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText;
new Function('require','module','exports',compiled)(name=>{
  if(name==='@tauri-apps/api/core')return {invoke:()=>{throw Error('singleton must not call native in this test');}};
  if(name==='@/services/indexedDBService')return {indexedDBService:{}};
  return fromFrontend(name);
},mod,mod.exports);
const {InterviewExit}=mod.exports;
const request=id=>({request_id:id,phase:'requested',restart:false});
const fixture=()=>({meetings:[{meetingId:'test',title:'Interview practice'}],transcripts:[{text:'synthetic speech'}]});
function setup(initialSuspended=false){
  const calls=[];let suspended=initialSuspended,cache=fixture(),fail='',prepareGate=null,cancelGate=null;
  const invoke=async(name,args)=>{
    calls.push(name);
    assert.equal(typeof args.requestId,'number');
    if(name===fail)throw Error('synthetic '+name+' failure');
    if(name==='interview_exit_prepare' && prepareGate)await prepareGate;
    if(name==='interview_exit_cancel' && cancelGate)await cancelGate;
    if(name==='interview_exit_archive'){assert.deepEqual(args.recovery,cache);return '/synthetic/Trash/exit-archive';}
  };
  const service={
    recordingWritesSuspended:()=>suspended,
    suspendRecordingWrites(){suspended=true;calls.push('suspend');},
    resumeRecordingWrites(){suspended=false;calls.push('resume');},
    async snapshotInterviewHistory(){calls.push('snapshot');assert.equal(suspended,true);if(fail==='snapshot')throw Error('synthetic snapshot failure');return structuredClone(cache);},
    async removeArchivedInterviewHistory(saved){calls.push('remove');assert.equal(suspended,true);assert.deepEqual(saved,cache);if(fail==='remove')throw Error('synthetic concurrent cache change');cache={meetings:[],transcripts:[]};}
  };
  return {flow:new InterviewExit({invoke,cache:service}),calls,fail(value){fail=value;},gate(value){prepareGate=value;},cancelGate(value){cancelGate=value;},suspended:()=>suspended,cache:()=>cache};
}
async function checkComponent(){
  const React=fromFrontend('react');const {create,act}=fromFrontend('react-test-renderer');
  const s=setup();s.fail('interview_exit_prepare');
  let listener,unlistened=0,resolveStatus;
  const pendingStatus=new Promise(resolve=>resolveStatus=resolve);
  const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/ExitCleanup.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX}}).outputText;
  const component={exports:{}};
  new Function('require','module','exports',code)(name=>{
    if(name==='@tauri-apps/api/core')return {isTauri:()=>true,invoke:async command=>{assert.equal(command,'interview_exit_status');return pendingStatus;}};
    if(name==='@tauri-apps/api/event')return {listen:async(name,fn)=>{assert.equal(name,'interview-exit-requested');listener=fn;return()=>unlistened++;}};
    if(name==='@/lib/interview-exit')return {interviewExit:s.flow};
    if(name==='@/components/ui/button')return {Button:props=>React.createElement('button',props)};
    if(name==='@/components/ui/dialog')return {Dialog:({open,children})=>open?React.createElement('section',null,children):null,DialogContent:props=>React.createElement('article',props),DialogHeader:props=>React.createElement('header',props),DialogTitle:props=>React.createElement('h1',props),DialogDescription:props=>React.createElement('p',props)};
    return fromFrontend(name);
  },component,component.exports);
  const text=node=>typeof node==='string'?node:(node?.children||[]).map(text).join('');
  let tree;await act(async()=>{tree=create(React.createElement(component.exports.ExitCleanup));});
  assert.equal(tree.toJSON(),null);
  await act(async()=>listener({payload:request(10)}));
  assert.ok(text(tree.toJSON()).includes('synthetic interview_exit_prepare failure'));
  await act(async()=>resolveStatus(null));assert.ok(text(tree.toJSON()).includes('退出未完成'),'late empty startup status hid failed quit');
  let prevented=0;tree.root.findByType('article').props.onEscapeKeyDown({preventDefault:()=>prevented++});
  assert.equal(prevented,1);
  await act(async()=>tree.root.findAllByType('button').find(n=>text(n)==='取消退出，返回应用').props.onClick());
  assert.equal(tree.toJSON(),null);assert.equal(s.flow.pending(),false);
  await act(async()=>tree.unmount());assert.equal(unlistened,1);
}
async function checkStorageGeneration(){
  let generation=4,pending=false,release,gate=null;const calls=[];
  const storage={exports:{}};
  const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/services/storageService.ts'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText;
  new Function('require','module','exports',code)(name=>{
    if(name==='@/lib/interview-exit')return {interviewExit:{pending:()=>pending,generation:()=>generation}};
    if(name==='@tauri-apps/api/core')return {invoke:async(command,args)=>{
      calls.push({command,args});
      if(command==='interview_exit_generation')return gate?await gate:35;
      assert.equal(command,'api_save_transcript');assert.equal(args.exitGeneration,35);return {meeting_id:'synthetic-new'};
    }};
    return fromFrontend(name);
  },storage,storage.exports);
  const service=storage.exports.storageService;
  // Native and renderer generations need not match after a renderer reload.
  assert.equal((await service.saveMeeting('fresh',[],null)).meeting_id,'synthetic-new');
  const before=calls.length;
  await assert.rejects(service.saveMeeting('stale',[],null,3));assert.equal(calls.length,before);
  gate=new Promise(resolve=>release=resolve);
  const delayed=service.saveMeeting('delayed',[],null,4);
  generation=5;release(35);await assert.rejects(delayed);
  assert.equal(calls.filter(c=>c.command==='api_save_transcript').length,1,'late native generation query revived old save');
  pending=true;gate=null;await assert.rejects(service.saveMeeting('during quit',[],null));
  assert.equal(calls.filter(c=>c.command==='api_save_transcript').length,1);
}
(async()=>{
  const s=setup();let release;s.gate(new Promise(resolve=>release=resolve));
  let sawPending=false;s.flow.subscribe(()=>{if(s.flow.pending())sawPending=true;});
  const running=s.flow.request(request(1));
  assert.equal(sawPending,true);assert.equal(s.flow.getSnapshot().busy,true);assert.equal(s.suspended(),true);
  await s.flow.request(request(1));await s.flow.retry();await s.flow.cancel();
  assert.deepEqual(s.calls,['suspend','interview_exit_prepare'],'duplicate quit or cancel during work ran another operation');
  release();await running;
  assert.deepEqual(s.calls,['suspend','interview_exit_prepare','snapshot','interview_exit_archive','remove','interview_exit_finish']);
  assert.equal(s.cache().transcripts.length,0);assert.equal(s.suspended(),true);
  assert.equal(s.flow.getSnapshot().error,'');

  for(const failure of ['interview_exit_prepare','snapshot','interview_exit_archive','remove','interview_exit_finish']){
    const f=setup();f.fail(failure);await f.flow.request(request(1));
    assert.equal(f.flow.getSnapshot().busy,false);assert.equal(f.flow.pending(),true);assert.ok(f.flow.getSnapshot().error.includes('synthetic'));
    if(failure!=='interview_exit_finish')assert.ok(!f.calls.includes('interview_exit_finish'),'quit was approved after '+failure+' failed');
    if(['interview_exit_prepare','snapshot','interview_exit_archive'].includes(failure))assert.ok(!f.calls.includes('remove'),'cache cleared without a successful archive');
    if(failure==='remove')assert.ok(f.flow.getSnapshot().error.includes('/synthetic/Trash/exit-archive'));
    const before=f.calls.length;await f.flow.request(request(1));assert.equal(f.calls.length,before,'failed quit silently retried');
    await f.flow.cancel();assert.equal(f.flow.pending(),false);assert.equal(f.suspended(),false);
    await f.flow.request(request(1));assert.equal(f.flow.pending(),false,'stale request reopened cancelled quit');
    assert.ok(!f.calls.some(c=>['answer_start','interview_audio_start','resume_recording','interview_session_save'].includes(c)),'exit implicitly started work or saved answers');
  }
  const retry=setup();retry.fail('remove');await retry.flow.request(request(1));
  assert.equal(retry.cache().transcripts.length,1);retry.fail('');await retry.flow.retry();
  assert.equal(retry.calls.filter(c=>c==='interview_exit_finish').length,1);assert.equal(retry.cache().transcripts.length,0);
  const cancelled=setup(true);cancelled.fail('snapshot');await cancelled.flow.request(request(1));await cancelled.flow.cancel();
  assert.equal(cancelled.suspended(),true,'cancel must not undo a preexisting write suspension');
  const recovered=setup();await recovered.flow.request({...request(5),phase:'failed'});
  assert.deepEqual(recovered.calls,['suspend']);assert.equal(recovered.flow.getSnapshot().busy,false);
  await recovered.flow.retry();assert.ok(recovered.calls.includes('interview_exit_finish'));
  const race=setup();race.fail('interview_exit_prepare');await race.flow.request(request(1));
  let releaseCancel;race.cancelGate(new Promise(resolve=>releaseCancel=resolve));
  const cancelling=race.flow.cancel();await race.flow.request(request(2));
  releaseCancel();await cancelling;
  assert.equal(race.flow.getSnapshot().requestId,2,'late cancel closed the newer quit');assert.equal(race.suspended(),true);
  race.cancelGate(null);await race.flow.cancel();assert.equal(race.suspended(),false,'new request lost original write state');
  await checkComponent();
  await checkStorageGeneration();
  console.log('PASS: exit waits for stop, archive and cache removal; every failure blocks quit; duplicate/stale notifications, explicit retry/cancel and prior cache suspension; no automatic AI or answer saving');
  console.log('PASS: actual exit dialog handles pending notification before startup status, exposes errors/cancel, prevents accidental dismissal and unregisters listener');
  console.log('PASS: storage rejects pre-quit work even after quit cancellation; native generation survives renderer reload and no stale save follows an awaited query');
})().catch(error=>{console.error(error);process.exitCode=1;});
