// Real session owners, deferred IPC, synthetic input only. Never opens devices or providers.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=req('react'),{create,act}=req('react-test-renderer'),ts=req('typescript');
const text=n=>typeof n==='string'?n:(n?.children||[]).map(text).join('');
const empty=()=>({version:1,history:[],transcripts:[],spoken_dialogue:[],current:{question:'',answer:''},calls:0,usage:'未知'});
const self={id:'self-1',role:'self',question:'合成的已说内容',suggestion:'',confirmed_answer:null,status:'done',sources:[],time:'00:01',timing:'',sequence_id:1,capture_session:1};
async function check(route){
  const calls=[],listeners=new Map(),cache={};let rows=[],gate=null,failSave=false,failClear=false,tree;
  const invoke=async(name,args)=>{
    calls.push({name,args});
    if(name==='interview_status')return {root:'',suggested_root:'',aliases:'',facts:'',key_configured:false,index_documents:0,index_chunks:0,documents:[],os:'windows'};
    if(name==='profile_import_status')return null;
    if(name==='interview_preferences_get')return {scenario:'general',role:'',organization:'',context:'',terminology:'',answer_language:'auto',transcription_language:'auto'};
    if(name==='answer_cancel'||name==='stop_recording')return;
    if(name==='session_clear'){if(failClear)throw Error('synthetic clear failure');return;}
    if(name==='interview_session_save'){
      assert.equal(args.confirmed,true);if(gate)await gate;if(failSave)throw Error('synthetic save failure');
      const snapshot=structuredClone(args.snapshot),row={id:rows.length+1,saved_at:new Date().toISOString(),turns:snapshot.history.length,calls:snapshot.calls,snapshot};rows.unshift(row);return row;
    }
    if(name==='interview_session_list')return {sessions:rows.map(({snapshot,...r})=>r),next_before:null};
    if(name==='interview_session_load'||name==='tingxian_session_preview'){if(gate)await gate;return structuredClone(rows.find(r=>r.id===args.id).snapshot);}
    throw Error('unexpected IPC: '+name);
  };
  function load(rel){
    if(cache[rel])return cache[rel];
    const source=fs.readFileSync(path.resolve(__dirname,'../frontend/src',rel),'utf8'),m={exports:{}};
    const js=ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX,esModuleInterop:true}}).outputText;
    new Function('require','module','exports','setInterval','clearInterval',js)(name=>{
      if(name==='@tauri-apps/api/core')return {invoke,isTauri:()=>true};
      if(name==='@tauri-apps/api/event')return {listen:async(event,fn)=>{listeners.set(event,fn);return()=>listeners.delete(event);}};
      if(name==='@/contexts/RecordingStateContext')return {useRecordingState:()=>({isRecording:false,isActive:false})};
      if(name==='@/contexts/TranscriptContext')return {useTranscripts:()=>({resetAfterHistoryClear(){}})};
      if(name==='@/services/indexedDBService')return {indexedDBService:{}};
      if(name==='@/lib/interview-exit')return {interviewExit:{pending:()=>false,subscribe:()=>()=>{},getSnapshot:()=>({requestId:null})}};
      if(name==='@/components/ui/button')return {Button:'button'};
      if(name==='@/components/ui/input')return {Input:'input'};
      if(name==='@/components/ui/choice-menu')return {ChoiceMenu:'choice-menu',ChoiceOption:'choice-option'};
      if(name==='next/link')return 'a';
      if(name==='lucide-react')return new Proxy({},{get:()=>()=>null});
      if(name.startsWith('@/components/interview/')){
        if(/\/(QuestionComposer|SavedSessionPreview|MeetingMode)$/.test(name))return load(name.slice(2)+'.tsx');
        return new Proxy({},{get:(_,key)=>props=>React.createElement('fixture-'+String(key),props)});
      }
      if(name.startsWith('@/lib/'))return load(name.slice(2)+'.ts');
      if(name.startsWith('./'))return load(path.posix.join(path.posix.dirname(rel),name)+'.ts');
      return req(name);
    },m,m.exports,()=>0,()=>{});return cache[rel]=m.exports;
  }
  const mount=()=>act(async()=>{tree=create(React.createElement(load(`app/${route}/page.tsx`).default));});
  const button=label=>tree.root.findAllByType('button').find(n=>text(n)===label);
  const click=label=>act(async()=>{const b=button(label);assert.ok(b,label);assert.ok(!b.props.disabled,label);await b.props.onClick();});
  const input=value=>act(async()=>{tree.root.findByProps({id:'question'}).props.onChange({target:{value}});});
  const value=()=>tree.root.findByProps({id:'question'}).props.value;
  const showHistory=()=>act(async()=>{listeners.get('tingxian-session-command')({payload:'history'});});
  const allow=()=>act(async()=>{tree.root.findAllByType('label').find(n=>text(n).includes('我已保存需要的内容')).findByType('input').props.onChange({target:{checked:true}});});
  await mount();await input('未提交：null、零、缺失字段');await showHistory();
  failSave=true;await click('结束会话并保留历史');
  assert.equal(value(),'未提交：null、零、缺失字段');assert.ok(text(tree.toJSON()).includes('synthetic save failure'));
  assert.equal(calls.filter(c=>c.name==='session_clear').length,0);
  failSave=false;let release;gate=new Promise(r=>release=r);
  let ending;await act(async()=>{ending=button('结束会话并保留历史').props.onClick();});
  await input('保存过程中新增的文字');gate=null;await act(async()=>{release();await ending;});
  assert.equal(value(),'保存过程中新增的文字','late save must not clear newer input');
  assert.equal(rows[0].turns,0);assert.equal(rows[0].snapshot.input_drafts.question,'未提交：null、零、缺失字段');
  assert.deepEqual(rows[0].snapshot.spoken_dialogue,[]);
  await showHistory();await allow();gate=new Promise(r=>release=r);
  let restoring;await act(async()=>{restoring=button('恢复这份会话').props.onClick();});
  await input('恢复期间新增的文字');gate=null;await act(async()=>{release();await restoring;});
  assert.equal(value(),'恢复期间新增的文字');assert.ok(text(tree.toJSON()).includes('当前内容发生变化'));
  await act(async()=>tree.unmount());await mount();assert.equal(value(),'','mount must not restore without selection');
  await showHistory();await click('查看详情');assert.ok(text(tree.toJSON()).includes('未提交的问题 · 未提交：null、零、缺失字段'));
  await click('恢复这份会话');assert.equal(value(),'未提交：null、零、缺失字段');assert.ok(button('发送').props.disabled);
  await showHistory();failClear=true;await click('结束会话并保留历史');
  assert.equal(value(),'未提交：null、零、缺失字段');assert.ok(text(tree.toJSON()).includes('synthetic clear failure'));
  failClear=false;
  // Restore a correction as an edit of the remapped self turn, never a new question.
  rows=[{id:100,saved_at:new Date().toISOString(),turns:1,calls:0,snapshot:{...empty(),history:[self],input_drafts:{question:'合成修正草稿',confirmed_answer:'尚未确认的本人回答',self_edit:{id:self.id,text:self.question}}}}];
  await act(async()=>tree.unmount());await mount();await showHistory();await click('恢复这份会话');
  assert.equal(value(),'合成修正草稿');assert.ok(button('保存修正'));await click('保存修正');assert.equal(value(),'');
  await showHistory();await click('保存当前会话到本机');
  assert.equal(rows[0].snapshot.history[0].question,'合成修正草稿');assert.equal(rows[0].snapshot.history[0].confirmed_answer,null);
  assert.equal(rows[0].snapshot.input_drafts.confirmed_answer,'尚未确认的本人回答');
  assert.equal(rows[0].snapshot.calls,0);assert.deepEqual(rows[0].snapshot.spoken_dialogue,[]);
  assert.ok(!calls.some(c=>/^(answer_start|interview_audio_start|resume_recording|interview_connection_test)$/.test(c.name)));
  const {parseSessionSnapshot,recoverSessionSnapshot}=load('lib/interview-session.ts');
  const legacy={...empty(),history:[self]};assert.deepEqual(parseSessionSnapshot(legacy),legacy);
  const recovered=recoverSessionSnapshot(rows[1].snapshot,'new');assert.equal(recovered.input_drafts.self_edit.id,recovered.history[0].id);
  assert.throws(()=>parseSessionSnapshot(empty()));
  for(const bad of [false,{}, {question:0,confirmed_answer:'',self_edit:null},{question:'q',confirmed_answer:'',self_edit:{id:3,text:'t'}}])assert.throws(()=>parseSessionSnapshot({...legacy,input_drafts:bad}));
  await act(async()=>tree.unmount());
  console.log(`PASS: ${route}: failed save/clear preserve input; input-only snapshots; deferred save/load races; fresh-mount recovery/preview; self-edit remapping; old payloads; zero capture/AI`);
}
(async()=>{await check('interview');await check('interview-workspace');})().catch(e=>{console.error(e);process.exitCode=1;});
