// Exercises the real page's controls with a strict native command contract.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const requireFrontend = require('node:module').createRequire(path.resolve(__dirname, '../frontend/package.json'));
const React = requireFrontend('react');
const { create, act } = requireFrontend('react-test-renderer');
const ts = requireFrontend('typescript');
const calls = [], listeners = new Map(), timers = [];
const conflict={id:'C-synthetic',topic:'本科 · 毕业日期',category:'date_difference',reason:'同一主题出现不同日期，请核对',left:{source_id:'S1',fact_version:null,path:'first.md',location:'段落1',quote:'本科毕业时间：2027年7月',value:'2027-07'},right:{source_id:'S2',fact_version:null,path:'second.md',location:'段落2',quote:'本科毕业时间：2026年7月',value:'2026-07'}};
let failStop=false, cancelOnStop=false;
let savedRows=[], savedId=0, failSave=false, corruptLoad=false;
let scene='interview',configuredKey=true,paused=false;
let requestConfig={model:'deepseek-flash',connect_timeout_secs:5,first_answer_warning_secs:8,total_timeout_secs:20};
const untestedConnection=()=>({check_id:null,status:'untested',model:null,started_at:null,finished_at:null,elapsed_ms:null,attempts:0,total_tokens:null,message:'尚未测试连接'});
let connection=untestedConnection(),holdConnection=false,releaseConnection=null;
let holdMicrophone=false,releaseMicrophone=null;
let failExport=false, exportedBlob=null, downloads=0, revoked=0;
global.document={createElement:tag=>{assert.equal(tag,'a');return {click(){downloads++;}};}};
URL.createObjectURL=blob=>{exportedBlob=blob;return 'blob:synthetic-export';};
URL.revokeObjectURL=url=>{assert.equal(url,'blob:synthetic-export');revoked++;};
let residueAvailable=false, failResidue=false, failCacheRemove=false, cacheSuspended=false;
let recoveryFixture={meetings:[],transcripts:[]};
let useHealth=false, nativeSession=100, healthSnapshot=null;
const nativeState=()=>({...native,...(useHealth?{health:{...healthSnapshot,recording:native.recording,microphone_enabled:native.enabled}}:{})});
const healthy=()=>({session_id:++nativeSession,recording:true,active:true,paused:false,system_started:true,microphone_enabled:true,active_for_ms:60000,system_frame_age_ms:0,microphone_frame_age_ms:0,system_signal_age_ms:null,asr_pending:0,asr_no_progress_ms:null,fault:null});
let native = { recording: false, enabled: false, epoch: 0 }, failClear = false, failIndexClear = false, indexDocuments = 2;
const invoke = async (name, args) => {
  calls.push({ name, args });
  if(name==='interview_preferences_save'){scene=args.preferences.scenario;return {...args.preferences};}
  if(name==='interview_preferences_get')return {scenario:scene,role:'',organization:'',context:'',terminology:'',answer_language:'auto',transcription_language:'auto'};
  if(name==='pause_recording'){paused=true;return;}
  if(name==='resume_recording'){paused=false;return;}
  if (name === 'profile_import_status') return null;
  if(name==='interview_request_config_get')return {...requestConfig};
  if(name==='interview_request_config_save'){requestConfig={...args.config};connection=untestedConnection();return {...requestConfig};}
  if(name==='interview_connection_status')return {...connection};
  if(name==='interview_connection_test'){if(holdConnection)await new Promise(resolve=>{releaseConnection=resolve;});assert.deepEqual(Object.keys(args).sort(),['checkId','confirmed']);assert.equal(args.confirmed,true);connection={...untestedConnection(),check_id:args.checkId,status:'passed',model:requestConfig.model,finished_at:new Date().toISOString(),elapsed_ms:10,attempts:1,total_tokens:9,message:'合成连接成功'};return {...connection};}
  if(name==='interview_connection_cancel')return {...connection};
  if(name==='interview_exit_prepare'){native.recording=false;return;}
  if(name==='interview_exit_archive')throw Error('synthetic exit Trash failure');
  if(name==='interview_exit_cancel')return;
  if(name==='interview_residue_status')return {active:native.recording,residue:{recording_folders:residueAvailable?1:0,unfinished_folders:residueAvailable?1:0,staged_archives:residueAvailable?1:0,database_meetings:residueAvailable?1:0,saved_sessions:savedRows.length}};
  if(name==='interview_residue_clear') {
    assert.equal(native.recording,false);assert.deepEqual(args.recovery,recoveryFixture);
    if(failResidue)throw Error('synthetic residue Trash failure');
    residueAvailable=false;return {archive_path:'/synthetic/Trash/residue',recording_folders:1,database_meetings:1,staged_archives:1,saved_sessions:0};
  }

  if (name === 'rag_search') {assert.ok(args.question);return {evidence:[],conflicts:[conflict]};}
  if (name === 'interview_config') {assert.equal(Object.hasOwn(args,'facts'),false,'ordinary config must not submit or confirm a fact draft');return;}
  if (name === 'interview_status') return { preferences:{scenario:scene}, root:'', suggested_root:'', model:requestConfig.model, aliases:'kept aliases', facts:'kept confirmed fact', key_configured:configuredKey, index_version:indexDocuments ? '1' : '2', index_documents:indexDocuments, index_chunks:indexDocuments ? 3 : 0, session_active:false };
  if (name === 'interview_audio_start') {
    assert.equal(typeof args?.recordMicrophone, 'boolean');
    assert.equal(native.recording, false, 'start while native capture still active');
    native = { recording:true, enabled:args.recordMicrophone, epoch:native.epoch+1 }; if(useHealth) healthSnapshot=healthy(); return nativeState();
  }
  if (name === 'stop_recording') {
    assert.equal(typeof args?.args?.save_path, 'string', 'native command requires args.save_path');
    if(failStop) throw Error('语音识别仍未退出，尚未完成停止');
    if(cancelOnStop) listeners.get('interview-transcription-cancelled')({payload:{native_session_id:healthSnapshot.session_id,message:'未完成的语音转写已取消，已显示文字保留。'}});
    native.recording = false; native.enabled = false; return;
  }
  if (name === 'get_recording_state') return { is_recording:native.recording };
  if (name === 'interview_microphone_set') {
    assert.equal(native.recording, true); native.enabled = args.enabled; native.epoch++; if(holdMicrophone)await new Promise(resolve=>{releaseMicrophone=resolve;});return nativeState();
  }
  if (name === 'interview_audio_level') return {system:0,microphone:0,microphone_state:nativeState()};
  if (name === 'profile_index_clear') {
    if (failIndexClear) throw Error('synthetic index Trash failure');
    const report={archive_path:indexDocuments ? '/synthetic/Trash/index' : null,documents:indexDocuments,chunks:indexDocuments ? 3 : 0,version:2};indexDocuments=0;return report;
  }
  if(name==='interview_session_save') {
    assert.equal(args.confirmed,true);if(failSave)throw Error('synthetic save failure');
    const row={id:++savedId,saved_at:new Date().toISOString(),turns:args.snapshot.history.length,calls:args.snapshot.calls};
    savedRows.unshift({...row,snapshot:JSON.parse(JSON.stringify(args.snapshot))});return row;
  }
  if(name==='interview_session_export') {
    if(failExport)throw Error('synthetic export failure');
    return {content:JSON.stringify({...args.snapshot,native_export_checked:true}),credentials_redacted:true};
  }
  if(name==='interview_session_list') return {sessions:savedRows.map(({snapshot,...row})=>row),next_before:null};
  if(name==='interview_session_load') {const row=savedRows.find(r=>r.id===args.id);if(!row)throw Error('not found');return corruptLoad?{version:999}:JSON.parse(JSON.stringify(row.snapshot));}
  if (name === 'interview_history_clear') {
    assert.equal(native.recording, false, 'clear before stopping capture');
    if (failClear) throw Error('synthetic Trash failure');
    const saved_sessions=savedRows.length;savedRows=[];
    return {archive_path:'/synthetic/Trash/archive',recording_folders:1,database_meetings:1,saved_sessions};
  }
  if (['answer_cancel','session_clear'].includes(name)) return;
  if (name === 'answer_start') {
    assert.ok(args.questionId);
    listeners.get('interview-answer')({payload:{question_id:args.questionId,revision:args.revision,event:{type:'request_sent',attempt:1}}});
    return;
  }
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
    if (name === '@/contexts/RecordingStateContext') return {useRecordingState:()=>({isRecording:native.recording,isActive:native.recording && !paused,isPaused:paused,isStopping:false})};
    if (name === '@/contexts/TranscriptContext') return {useTranscripts:()=>({resetAfterHistoryClear:()=>{}})};
    if (name === '@/services/indexedDBService') return {indexedDBService:{suspendRecordingWrites(){cacheSuspended=true;},resumeRecordingWrites(){cacheSuspended=false;},recordingWritesSuspended:()=>cacheSuspended,inspectInterviewHistory:async()=>({meetings:recoveryFixture.meetings.length,transcripts:recoveryFixture.transcripts.length}),snapshotInterviewHistory:async()=>JSON.parse(JSON.stringify(recoveryFixture)),removeArchivedInterviewHistory:async()=>{if(failCacheRemove)throw Error('synthetic changed cache');recoveryFixture={meetings:[],transcripts:[]};}}};
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
  await act(async()=>{tree=create(React.createElement(load('app/interview/page.tsx').default));});
  const button = label => tree.root.findAllByType('button').find(node=>text(node)===label);
  const click = async label => {const b=button(label);assert.ok(b, label);assert.equal(!!b.props.disabled,false,label+' disabled');await act(async()=>{await b.props.onClick();});};
  const check = async (label,value) => {
    const node = tree.root.findAllByType('label').find(node=>text(node).includes(label));assert.ok(node,label);
    await act(async()=>{await node.findByType('input').props.onChange({target:{checked:value}});});
  };
  const emit = async payload => {await act(async()=>{listeners.get('transcript-update')({payload});});};
  assert.ok(text(tree.toJSON()).includes('阅读模式 · 保留当前位置'));
  await click('跟随最新对话');assert.ok(button('停止跟随'));
  assert.equal(calls.filter(c=>c.name==='interview_connection_test').length,0,'mount must not test the provider automatically');
  await click('测试 DeepSeek 连接');assert.ok(text(tree.toJSON()).includes('上次测试成功'));assert.equal(calls.filter(c=>c.name==='answer_start').length,0);
  // Connection checking disables both button and shortcut, including the pre-render window.
  await check('允许向 DeepSeek',true);
  await act(async()=>tree.root.findByProps({id:'question'}).props.onChange({target:{value:'连接检查期间保留的问题'}}));
  holdConnection=true;
  const shortcut=()=>tree.root.findByProps({id:'question'}).props.onKeyDown({key:'Enter',metaKey:true,nativeEvent:{},preventDefault(){}});
  await act(async()=>{button('测试 DeepSeek 连接').props.onClick();shortcut();});
  assert.equal(button('发送').props.disabled,true);
  await act(async()=>shortcut());assert.equal(calls.filter(c=>c.name==='answer_start').length,0);
  assert.equal(tree.root.findByProps({id:'question'}).props.value,'连接检查期间保留的问题');
  holdConnection=false;await act(async()=>releaseConnection());assert.equal(button('发送').props.disabled,false);
  console.log('PASS: connection check blocks shortcuts before and after render without discarding draft');
  await click('资料与设置');
  await act(async()=>tree.root.findByProps({'aria-label':'回答模型 ID'}).props.onChange({target:{value:'synthetic-model'}}));
  await click('保存回答请求设置');assert.ok(text(tree.toJSON()).includes('DeepSeek · synthetic-model'));
  assert.ok(text(tree.toJSON()).includes('API 连接：尚未测试'),'model config changes must invalidate previous check');
  await click('恢复默认请求设置');assert.equal(requestConfig.model,'synthetic-model','default draft must not change persisted config');
  await click('保存回答请求设置');assert.ok(text(tree.toJSON()).includes('DeepSeek · deepseek-flash'));
  await act(async()=>{tree.root.findByProps({'aria-label':'资料根目录'}).props.onChange({target:{value:'/synthetic/next-import'}});});
  await act(async()=>{tree.root.findByProps({placeholder:'只填写本人核对过的事实与来源，不粘贴 AI 建议。'}).props.onChange({target:{value:'unsaved fact draft'}});});
  await click('刷新已导入资料');
  assert.equal(tree.root.findByProps({'aria-label':'资料根目录'}).props.value,'/synthetic/next-import','review refresh must not retarget the next import');
  assert.equal(tree.root.findByProps({placeholder:'只填写本人核对过的事实与来源，不粘贴 AI 建议。'}).props.value,'unsaved fact draft','review refresh must not overwrite unsaved facts');
  await click('保存配置');
  assert.equal(tree.root.findByProps({placeholder:'只填写本人核对过的事实与来源，不粘贴 AI 建议。'}).props.value,'unsaved fact draft','saving aliases/key must leave the unsaved fact draft alone');
  await click('资料与设置');
  await act(async()=>tree.root.findByProps({id:'question'}).props.onChange({target:{value:'本科毕业时间'}}));
  await click('本地检索');
  assert.ok(text(tree.toJSON()).includes('本科毕业时间：2027年7月'));assert.ok(text(tree.toJSON()).includes('本科毕业时间：2026年7月'));
  await click('打开资料与事实核对');assert.ok(text(tree.toJSON()).includes('已确认事实更正'));
  assert.equal(calls.filter(c=>c.name==='profile_facts_correct'||c.name==='answer_start').length,0,'reviewing a difference must not confirm facts or invoke AI');
  await click('资料与设置');
  await check('已获允许使用转写',true);await check('允许向 DeepSeek',true);
  await check('记录我的声音',false);
  await click('开始持续聆听与回答');assert.equal(native.enabled,false);
  await check('记录我的声音',true);assert.equal(native.enabled,true);
  const firstEpoch=native.epoch;
  const own=(id,epoch,words)=>({confidence:null,sequence_id:id,source:'Microphone',microphone_epoch:epoch,text:words,is_partial:false,timestamp:'00:01',audio_start_time:0,audio_end_time:1});
  await emit(own(1,firstEpoch,'自己的答案保留'));assert.ok(text(tree.toJSON()).includes('自己的答案保留'));
  await check('允许向 DeepSeek',false);
  await click('纠正我的回答');
  await act(async()=>tree.root.findByProps({id:'question'}).props.onChange({target:{value:'保留的手动修正草稿'}}));
  await emit(own(1,firstEpoch,'自己的答案保留，整句复核更新'));
  await click('保存修正');
  assert.match(text(tree.toJSON()),/已更新或被移除/);
  assert.equal(tree.root.findByProps({id:'question'}).props.value,'保留的手动修正草稿');
  assert.ok(text(tree.toJSON()).includes('自己的答案保留，整句复核更新'));
  await click('取消修改');assert.equal(tree.root.findByProps({id:'question'}).props.value,'');
  await click('纠正我的回答');
  const correctedSelf='自己的答案保留：我会分别验证 JSON Schema 和业务语义';
  await act(async()=>tree.root.findByProps({id:'question'}).props.onChange({target:{value:correctedSelf}}));
  await click('保存修正');
  assert.ok(text(tree.toJSON()).includes(correctedSelf));assert.ok(text(tree.toJSON()).includes('本人回答已修正，未调用 AI'));
  assert.equal(tree.root.findByProps({id:'question'}).props.value,'');assert.equal(calls.filter(c=>c.name==='answer_start').length,0);
  await check('允许向 DeepSeek',true);
  const beforeExportAI=calls.filter(c=>c.name==='answer_start').length;
  failExport=true;await click('导出本次记录');assert.equal(downloads,0);assert.ok(text(tree.toJSON()).includes('synthetic export failure'));
  assert.ok(text(tree.toJSON()).includes('自己的答案保留'),'export failure cleared history');
  failExport=false;await click('导出本次记录');assert.equal(downloads,1);assert.equal(revoked,1);
  const downloaded=JSON.parse(await exportedBlob.text());assert.equal(downloaded.native_export_checked,true,'download bypassed native filtering');
  assert.equal(downloaded.usage_ledger.version,1,'export omitted observed usage ledger');
  assert.ok(downloaded.history.some(t=>t.question===correctedSelf));assert.equal(downloaded.calls,0);
  assert.ok(downloaded.spoken_dialogue.some(t=>t.source==='Microphone' && t.text===correctedSelf),'local edit was not saved in future dialogue context');
  assert.ok(downloaded.transcripts.some(t=>t.text==='自己的答案保留，整句复核更新'),'local edit overwrote raw transcription');
  console.log('PASS: cloud-disabled self correction updates dialogue only, rejects stale edits, preserves draft/raw text and makes no AI request');
  assert.ok(text(tree.toJSON()).includes('API 凭证已隐藏'));assert.equal(calls.filter(c=>c.name==='answer_start').length,beforeExportAI);
  console.log('PASS: actual export uses native result, preserves full history, releases Blob URL, and fails without download/data loss/API');
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
  assert.ok(button('跟随最新对话'),'successful clear must restore default reading mode');
  assert.equal(native.recording,false);assert.ok(!text(tree.toJSON()).includes('自己的答案保留'));
  await click('开始持续聆听与回答');assert.equal(native.recording,true);
  assert.equal(calls.filter(c=>c.name==='stop_recording').length,4);
  const remote={confidence:null,sequence_id:100,source:'System',utterance_id:8,text:'如何区分 null 和零？',is_partial:false,timestamp:'00:02',audio_start_time:0,audio_end_time:2};
  await emit(remote);
  await act(async()=>{timers.find(t=>t.delay===100).callback();});
  assert.equal(calls.filter(c=>c.name==='answer_start').length,0,'unknown confidence is neither a stop nor a completion signal');
  await act(async()=>{listeners.get('interview-utterance-end')({payload:{source:'System',utterance_id:8,success:true,timing:{stable_text_unix_ms:Date.now(),audio_end_to_stable_ms:2500,vad_to_stable_ms:1000}}});});
  holdMicrophone=true;let microphoneChange;
  await act(async()=>{
    microphoneChange=tree.root.findAllByType('label').find(n=>text(n).includes('记录我的声音')).findByType('input').props.onChange({target:{checked:false}});
    timers.find(t=>t.delay===100).callback();
  });
  await act(async()=>timers.find(t=>t.delay===100).callback());
  assert.equal(calls.filter(c=>c.name==='answer_start').length,0,'busy operation sent/dequeued remote speech');
  holdMicrophone=false;await act(async()=>{releaseMicrophone();await microphoneChange;});
  await act(async()=>{timers.find(t=>t.delay===100).callback();});
  const request=calls.filter(c=>c.name==='answer_start');assert.equal(request.length,1);
  assert.equal(request[0].args.question,remote.text,'queued complete speech was lost during busy operation');
  await check('记录我的声音',true);
  console.log('PASS: busy operation holds completed remote speech even before render, then submits it exactly once');
  const stream=async event=>{await act(async()=>{listeners.get('interview-answer')({payload:{question_id:request[0].args.questionId,revision:request[0].args.revision,event}});});};
  await stream({type:'sources',evidence:[],conflicts:[conflict],retrieval_ms:3,fact_version:3});await stream({type:'request_sent',attempt:1});
  await stream({type:'request_sent',attempt:1});await stream({type:'request_sent',attempt:2});
  await act(async()=>listeners.get('interview-answer')({payload:{question_id:request[0].args.questionId,revision:request[0].args.revision+99,event:{type:'delta',text:'STALE_REVISION_MUST_NOT_APPEAR'}}}));
  await act(async()=>listeners.get('interview-answer')({payload:{question_id:request[0].args.questionId,revision:request[0].args.revision+99,event:{type:'done',usage:{total_tokens:999}}}}));
  assert.ok(!text(tree.toJSON()).includes('STALE_REVISION_MUST_NOT_APPEAR'));
  await act(async()=>listeners.get('interview-answer')({payload:{question_id:'stale-question',revision:0,event:{type:'sources',evidence:[],conflicts:[{...conflict,topic:'STALE_CONFLICT_MUST_NOT_APPEAR'}]}}}));
  assert.ok(!text(tree.toJSON()).includes('STALE_CONFLICT_MUST_NOT_APPEAR'));
  assert.ok(text(tree.toJSON()).includes('事实卡 v3'),'answer must retain the fact version used for that request');
  await stream({type:'delta',text:'建议回答：'});assert.ok(!text(tree.toJSON()).includes('提交→建议'));
  await stream({type:'warning',message:'SYNTHETIC_WAITING_FOR_USEFUL_ANSWER'});assert.ok(text(tree.toJSON()).includes('SYNTHETIC_WAITING_FOR_USEFUL_ANSWER'));
  await stream({type:'delta',text:'null 表示无值，零是具体数值。应分别验证。'});
  assert.ok(!text(tree.toJSON()).includes('SYNTHETIC_WAITING_FOR_USEFUL_ANSWER'),'useful answer must dismiss obsolete waiting message');
  await stream({type:'warning',message:'SYNTHETIC_WARNING_BEFORE_COMPLETION'});
  await stream({type:'done',elapsed_ms:1000,first_text_ms:100,usage:{total_tokens:50}});
  await stream({type:'done',elapsed_ms:1000,first_text_ms:100,usage:{total_tokens:50}});
  assert.ok(text(tree.toJSON()).includes('已知累计 50 tokens · 观测到 2 次发送 · 1 次发送用量未知'));
  assert.ok(!text(tree.toJSON()).includes('SYNTHETIC_WARNING_BEFORE_COMPLETION'),'finished requests must hide waiting status');
  assert.ok(text(tree.toJSON()).includes('说完→首句约'));assert.ok(text(tree.toJSON()).includes('提交→建议'));
  await emit(own(101,native.epoch,'我会分别验证'));await act(async()=>{timers.find(t=>t.delay===100).callback();});
  assert.equal(calls.filter(c=>c.name==='answer_start').length,1,'own speech must not request an answer after timing instrumentation');
  await click('资料与设置');
  assert.equal(button('移入废纸篓并清空索引').props.disabled,true,'active interview must lock index clearing');
  await click('结束会话并保留历史');
  const historyClearsBefore=calls.filter(c=>c.name==='interview_history_clear').length;
  failIndexClear=true;await click('移入废纸篓并清空索引');
  assert.ok(text(tree.toJSON()).includes('synthetic index Trash failure'));
  assert.ok(text(tree.toJSON()).includes('已索引 2 份资料'));
  assert.ok(text(tree.toJSON()).includes('我会分别验证'),'failed index clear erased conversation');
  failIndexClear=false;await click('移入废纸篓并清空索引');
  assert.ok(text(tree.toJSON()).includes('已索引 0 份资料'));
  assert.ok(text(tree.toJSON()).includes('/synthetic/Trash/index'));
  assert.ok(text(tree.toJSON()).includes('我会分别验证'),'successful index clear erased conversation');
  assert.ok(text(tree.toJSON()).includes('本次调用 1 /'),'index clear reset session call count');
  assert.ok(text(tree.toJSON()).includes('本科毕业时间：2027年7月'),'index clear must retain the historical answer conflict snapshot');
  assert.equal(tree.root.findByProps({placeholder:'只填写本人核对过的事实与来源，不粘贴 AI 建议。'}).props.value,'kept confirmed fact');
  assert.equal(calls.filter(c=>c.name==='interview_history_clear').length,historyClearsBefore);
  await click('移入废纸篓并清空索引');assert.ok(text(tree.toJSON()).includes('资料索引已为空'));
  // Exercise the actual page with modern, session-scoped native health.
  await click('资料与设置'); useHealth=true;
  await click('开始持续聆听与回答');
  const poll=async()=>{await act(async()=>{timers.find(t=>t.delay===500).callback();});};
  const tick=async()=>{await act(async()=>{timers.find(t=>t.delay===100).callback();});};
  const scoped=async (id,words,session=healthSnapshot.session_id)=>{
    await emit({...remote,sequence_id:id,utterance_id:id,native_session_id:session,text:words});
    await act(async()=>listeners.get('interview-utterance-end')({payload:{source:'System',utterance_id:id,native_session_id:session,success:true}}));
  };
  const answers=()=>calls.filter(c=>c.name==='answer_start');
  await poll(); assert.ok(text(tree.toJSON()).includes('静音时属于正常情况'));
  await scoped(201,'故障前排队的问题');
  const beforeFault=answers().length;
  const fault={session_id:healthSnapshot.session_id,sequence:1,code:'capture_error'};
  healthSnapshot={...healthSnapshot,fault,system_frame_age_ms:null};
  // Deliberately fire scheduler in the same React batch as the fault.
  await act(async()=>{listeners.get('interview-audio-fault')({payload:fault});timers.find(t=>t.delay===100).callback();});
  assert.equal(answers().length,beforeFault,'fault/timer race sent a queued question');
  assert.ok(text(tree.toJSON()).includes('音频采集报错'));
  await poll();assert.equal(button('确认声音恢复，继续自动回答').props.disabled,true);
  await scoped(202,'故障期间的发言');await tick();assert.equal(answers().length,beforeFault);
  // Both manual input and local search remain available while audio is faulty.
  await act(async()=>tree.root.findByProps({id:'question'}).props.onChange({target:{value:'手动问题'}}));
  await click('本地检索');assert.ok(text(tree.toJSON()).includes('音频采集报错'),'local notice must not erase health alert');
  const send=tree.root.findAllByType('button').find(n=>text(n).includes('发送') && !text(n).includes('取消'));
  assert.ok(send,'manual send control');await act(async()=>send.props.onClick());assert.equal(answers().length,beforeFault+1);
  const finish=async()=>{const req=answers().at(-1);await act(async()=>listeners.get('interview-answer')({payload:{question_id:req.args.questionId,revision:req.args.revision,event:{type:'done'}}}));};
  await finish();
  healthSnapshot={...healthSnapshot,system_frame_age_ms:0};await poll();
  assert.equal(button('确认声音恢复，继续自动回答').props.disabled,false);
  await click('确认声音恢复，继续自动回答');await tick();assert.equal(answers().length,beforeFault+1,'recovery must not replay the old queue');
  await scoped(203,'恢复后的新问题');await tick();assert.equal(answers().length,beforeFault+2);await finish();
  await emit({...own(204,native.epoch,'我的恢复回答'),native_session_id:healthSnapshot.session_id});await tick();assert.equal(answers().length,beforeFault+2);
  healthSnapshot={...healthSnapshot,asr_pending:4,asr_no_progress_ms:1000};await poll();assert.ok(text(tree.toJSON()).includes('转写积压'));
  healthSnapshot={...healthSnapshot,asr_pending:0,asr_no_progress_ms:null};await poll();assert.ok(button('确认声音恢复，继续自动回答'),'queue drain must remain paused');
  const old=healthSnapshot.session_id;await click('停止采集');await click('开始持续聆听与回答');
  await act(async()=>listeners.get('interview-audio-fault')({payload:{session_id:old,sequence:99,code:'capture_error'}}));
  await scoped(205,'旧会话迟到的文字不可见',old);
  assert.ok(!text(tree.toJSON()).includes('旧会话迟到的文字不可见'));
  assert.equal(button('确认声音恢复，继续自动回答'),undefined,'old fault paused new capture');
  await scoped(206,'重开后新的问题');await tick();assert.equal(answers().length,beforeFault+3);
  await finish();
  await emit({...remote,sequence_id:207,utterance_id:207,native_session_id:healthSnapshot.session_id,text:'尚未说完的段落'});
  assert.ok(text(tree.toJSON()).includes('对方仍在说话'));
  const overflow={session_id:healthSnapshot.session_id,sequence:1,code:'asr_overflow'};
  healthSnapshot={...healthSnapshot,active:false,fault:overflow};
  await act(async()=>{listeners.get('interview-audio-fault')({payload:overflow});timers.find(t=>t.delay===100).callback();});
  await poll();assert.ok(text(tree.toJSON()).includes('文字可能不完整'));
  assert.ok(!text(tree.toJSON()).includes('对方仍在说话'),'interrupted speech must not stay labelled as live listening');
  await emit({...remote,sequence_id:208,utterance_id:207,native_session_id:healthSnapshot.session_id,text:'仍保留已接收的转写'});
  assert.ok(text(tree.toJSON()).includes('仍保留已接收的转写'));
  assert.ok(!text(tree.toJSON()).includes('对方仍在说话'));
  assert.equal(button('确认声音恢复，继续自动回答').props.disabled,true);
  await tick();assert.equal(answers().length,beforeFault+3,'overflow must not answer an incomplete utterance');
  await click('停止采集');await click('开始持续聆听与回答');
  await scoped(209,'容量故障重开后的完整问题');await tick();assert.equal(answers().length,beforeFault+4);
  await finish();failStop=true;await click('停止采集');
  assert.ok(text(tree.toJSON()).includes('尚未完成停止'));assert.equal(button('开始持续聆听与回答').props.disabled,true);
  assert.ok(text(tree.toJSON()).includes('容量故障重开后的完整问题'),'failed Stop must preserve conversation');
  failStop=false;cancelOnStop=true;const cancelledSession=healthSnapshot.session_id;
  await emit({...remote,sequence_id:210,utterance_id:210,native_session_id:cancelledSession,text:'停止时尚未完成的发言'});
  await click('停止采集');assert.ok(text(tree.toJSON()).includes('未完成的语音转写已取消'));
  assert.ok(!text(tree.toJSON()).includes('对方仍在说话'));
  cancelOnStop=false;await click('开始持续聆听与回答');
  await act(async()=>listeners.get('interview-transcription-cancelled')({payload:{native_session_id:cancelledSession,message:'旧取消事件不可见'}}));
  assert.ok(!text(tree.toJSON()).includes('旧取消事件不可见'));


  const beforeSaveCalls=calls.filter(c=>c.name==='answer_start').length;const priorSaved=savedRows.length;
  failSave=true;await click('保存当前会话到本机');assert.ok(text(tree.toJSON()).includes('synthetic save failure'));assert.equal(savedRows.length,priorSaved);
  failSave=false;await click('保存当前会话到本机');assert.equal(savedRows.length,priorSaved+1);
  assert.equal(savedRows[0].snapshot.usage_ledger.version,1);
  assert.equal(savedRows[0].snapshot.usage_ledger.requests.filter(r=>r.accepted).length,savedRows[0].snapshot.calls);
  await click('结束会话并保留历史');assert.equal(native.recording,false);assert.equal(savedRows.length,priorSaved+2);
  await click('查看已保存会话');assert.equal(button('恢复这份会话').props.disabled,true,'unsaved display must not be silently replaced');
  await check('我已保存需要的内容',true);corruptLoad=true;await click('恢复这份会话');assert.ok(text(tree.toJSON()).includes('容量故障重开后的完整问题'),'bad restore replaced existing history');
  corruptLoad=false;await click('跟随最新对话');await click('恢复这份会话');assert.ok(text(tree.toJSON()).includes('已恢复的历史建议'));
  assert.ok(button('跟随最新对话'),'restoring a snapshot must not inherit automatic scrolling');
  assert.equal(calls.filter(c=>c.name==='answer_start').length,beforeSaveCalls);
  const savedCount=savedRows.length;const explicitlySaved=JSON.parse(JSON.stringify(savedRows[0]));
  await act(async()=>tree.unmount());
  await act(async()=>{tree=create(React.createElement(load('app/interview/page.tsx').default));});
  assert.equal(savedRows.length,savedCount,'mount must not save automatically');
  assert.ok(!text(tree.toJSON()).includes('容量故障重开后的完整问题'),'mount must not load body without selection');
  await click('查看已保存会话');corruptLoad=true;await click('恢复这份会话');
  assert.ok(text(tree.toJSON()).includes('格式无效'));assert.ok(!text(tree.toJSON()).includes('容量故障重开后的完整问题'));
  corruptLoad=false;await click('恢复这份会话');assert.ok(text(tree.toJSON()).includes('容量故障重开后的完整问题'));
  assert.equal(native.recording,false);assert.equal(calls.filter(c=>c.name==='answer_start').length,beforeSaveCalls);
  assert.ok(!text(tree.toJSON()).includes('对方仍在说话'));
  await check('已获允许使用转写',true);await check('允许向 DeepSeek',true);await click('开始持续聆听与回答');
  await emit({...own(204,native.epoch,'新会话本人说明'),native_session_id:healthSnapshot.session_id});
  assert.ok(text(tree.toJSON()).includes('我的恢复回答'));assert.ok(text(tree.toJSON()).includes('新会话本人说明'));
  await scoped(999,'恢复后追问');await act(async()=>timers.filter(t=>t.delay===100).at(-1).callback());
  const afterRestore=calls.filter(c=>c.name==='answer_start');assert.equal(afterRestore.length,beforeSaveCalls+1);
  assert.ok(afterRestore.at(-1).args.dialogue.some(t=>t.text==='我的恢复回答'));
  assert.ok(afterRestore.at(-1).args.dialogue.some(t=>t.text==='新会话本人说明'));
  assert.ok(!afterRestore.at(-1).args.dialogue.some(t=>t.text.startsWith('建议回答')));
  failClear=true;await click('清空历史，重新开始');assert.equal(savedRows.length,savedCount);
  failClear=false;await click('清空历史，重新开始');assert.equal(savedRows.length,0);
  await stream({type:'done',usage:{total_tokens:999}});
  assert.ok(text(tree.toJSON()).includes('累计用量：尚无调用记录'),'late usage revived a cleared session');
  await click('查看已保存会话');assert.ok(text(tree.toJSON()).includes('暂无已保存会话'));
  const legacySaved=JSON.parse(JSON.stringify(explicitlySaved));delete legacySaved.snapshot.usage_ledger;savedRows=[legacySaved];
  const beforeLegacyCalls=calls.filter(c=>c.name==='answer_start').length;
  await click('查看已保存会话');await click('恢复这份会话');
  assert.ok(text(tree.toJSON()).includes(`旧记录 ${legacySaved.snapshot.calls} 次用量未知`));
  assert.equal(calls.filter(c=>c.name==='answer_start').length,beforeLegacyCalls);
  await act(async()=>tree.unmount());
  savedRows=[explicitlySaved];residueAvailable=true;cacheSuspended=false;
  recoveryFixture={meetings:[{meetingId:'old',title:'Interview practice'}],transcripts:[{id:1,text:'old cached speech'}]};
  const residueCalls=calls.filter(c=>c.name==='interview_residue_clear').length;
  const aiBeforeResidue=calls.filter(c=>c.name==='answer_start').length;
  await act(async()=>{tree=create(React.createElement(load('app/interview/page.tsx').default));});
  assert.ok(text(tree.toJSON()).includes('检测到上次采集留下的记录'));
  assert.equal(calls.filter(c=>c.name==='interview_residue_clear').length,residueCalls,'inspection must not clear automatically');
  await click('暂时保留这些记录');assert.ok(!text(tree.toJSON()).includes('检测到上次采集留下的记录'));
  await click('重新检查上次残留');failResidue=true;await click('清理上次残留，保留已保存会话');
  assert.ok(text(tree.toJSON()).includes('synthetic residue Trash failure'));assert.equal(cacheSuspended,false);assert.equal(savedRows.length,1);assert.equal(recoveryFixture.transcripts.length,1);
  failResidue=false;failCacheRemove=true;await click('清理上次残留，保留已保存会话');
  assert.ok(text(tree.toJSON()).includes('/synthetic/Trash/residue'),'cache failure must point to existing backup');assert.equal(savedRows.length,1);assert.equal(recoveryFixture.transcripts.length,1);
  failCacheRemove=false;await click('清理上次残留，保留已保存会话');
  assert.equal(savedRows.length,1);assert.equal(recoveryFixture.transcripts.length,0);assert.equal(cacheSuspended,false);
  assert.ok(!text(tree.toJSON()).includes('检测到上次采集留下的记录'));assert.equal(calls.filter(c=>c.name==='answer_start').length,aiBeforeResidue);
  await click('查看已保存会话');assert.ok(button('恢复这份会话'));
  await click('恢复这份会话');
  const beforeExitAI=calls.filter(c=>c.name==='answer_start').length;
  const beforeExitHistory=text(tree.toJSON());
  const exit=load('lib/interview-exit.ts').interviewExit;
  await act(async()=>exit.request({request_id:1,phase:'requested',restart:false}));
  assert.equal(exit.pending(),true);assert.ok(exit.getSnapshot().error.includes('synthetic exit Trash failure'));
  assert.ok(text(tree.toJSON()).includes('自动回答已暂停'));
  await tick();assert.equal(calls.filter(c=>c.name==='answer_start').length,beforeExitAI);
  assert.ok(text(tree.toJSON()).includes('已恢复的历史建议') && beforeExitHistory.includes('已恢复的历史建议'),'quit failure erased current answers');
  await act(async()=>exit.cancel());assert.equal(exit.pending(),false);
  await tick();assert.equal(calls.filter(c=>c.name==='answer_start').length,beforeExitAI,'cancel quit resumed AI automatically');
  assert.equal(savedRows.length,1);
  console.log('PASS: actual page pauses automatic replies for exit, preserves answers on failure, and does not restart AI when exit is cancelled');
  console.log('PASS: startup residue counts, explicit defer/recheck, no automatic deletion/API, Trash/cache failure recovery, temporary cleanup preserves saved sessions');
  console.log('PASS: cumulative usage separates retry uncertainty, deduplicates done events, isolates revisions/cleared sessions, persists/export metadata and restores legacy as unknown');
  console.log('PASS: explicit save failure/success, end/save, fresh mount/selective restore, corrupt payload, no replay/permissions, cross-session context, and clear includes saved snapshots');
  await act(async()=>tree.unmount());
  // Budget checks use valid legacy snapshots to reach the boundary without 80 synthetic requests.
  const budgetSaved=JSON.parse(JSON.stringify(explicitlySaved));delete budgetSaved.snapshot.usage_ledger;
  budgetSaved.snapshot.calls=74;savedRows=[budgetSaved];
  await act(async()=>{tree=create(React.createElement(load('app/interview/page.tsx').default));});
  assert.ok(text(tree.toJSON()).includes('本次调用 0 / 80'));
  await click('查看已保存会话');await click('恢复这份会话');
  assert.ok(text(tree.toJSON()).includes('本次调用 74 / 80'));assert.equal(button('继续 20 次'),undefined);
  await check('允许向 DeepSeek',true);
  await act(async()=>tree.root.findByProps({id:'question'}).props.onChange({target:{value:'合成第75次问题'}}));
  await click('发送');await finish();
  assert.ok(text(tree.toJSON()).includes('本轮还可请求 5 次建议回答'));
  await click('继续 20 次');assert.ok(text(tree.toJSON()).includes('本次调用 75 / 100'));
  assert.ok(!text(tree.toJSON()).includes('本轮还可请求 5 次建议回答'));
  await click('清空历史，重新开始');assert.ok(text(tree.toJSON()).includes('本次调用 0 / 80'));
  budgetSaved.snapshot.calls=79;savedRows=[budgetSaved];
  await click('查看已保存会话');await click('恢复这份会话');
  await check('已获允许使用转写',true);await check('允许向 DeepSeek',true);await click('开始持续聆听与回答');
  const latestTick=async()=>{await act(async()=>timers.filter(t=>t.delay===100).at(-1).callback());};
  const beforeBudget=answers().length;
  await scoped(8001,'合成第80次完整发言');await latestTick();assert.equal(answers().length,beforeBudget+1);
  assert.ok(text(tree.toJSON()).includes('本次调用 80 / 80'));
  const finalAllowed=answers().at(-1),cancelBeforeLimit=calls.filter(c=>c.name==='answer_cancel').length;
  await scoped(8002,'等额度增加后回答的整句');await latestTick();
  assert.equal(calls.filter(c=>c.name==='answer_cancel').length,cancelBeforeLimit,'budget cancelled an in-flight answer');
  await act(async()=>listeners.get('interview-answer')({payload:{question_id:finalAllowed.args.questionId,revision:finalAllowed.args.revision,event:{type:'delta',text:'建议回答：第80次仍可完整结束。'}}}));
  await finish();await latestTick();assert.equal(answers().length,beforeBudget+1);
  assert.ok(text(tree.toJSON()).includes('第80次仍可完整结束。'));assert.equal(native.recording,true);
  await emit({...own(8003,native.epoch,'额度已满时本人的回答'),native_session_id:healthSnapshot.session_id});await latestTick();assert.equal(answers().length,beforeBudget+1);
  await act(async()=>tree.root.findByProps({id:'question'}).props.onChange({target:{value:'额度满时不应发送的手动问题'}}));
  await click('发送');assert.equal(answers().length,beforeBudget+1);assert.equal(tree.root.findByProps({id:'question'}).props.value,'额度满时不应发送的手动问题');
  await act(async()=>tree.root.findAllByType('button').filter(n=>text(n)==='纠正我的回答').at(-1).props.onClick());
  await act(async()=>tree.root.findByProps({id:'question'}).props.onChange({target:{value:'额度已满时仍可本地纠正'}}));
  await click('保存修正');assert.equal(answers().length,beforeBudget+1);
  await click('继续 20 次');await check('对方说完后自动回答',true);await latestTick();
  assert.equal(answers().length,beforeBudget+2);assert.equal(answers().at(-1).args.question,'等额度增加后回答的整句');
  assert.ok(answers().at(-1).args.dialogue.some(t=>t.text==='额度已满时仍可本地纠正'));
  assert.ok(text(tree.toJSON()).includes('本次调用 81 / 100'));await finish();
  await click('停止采集');await act(async()=>tree.unmount());
  // Each new scene records without cloud consent; pause/resume must not opt into AI.
  for(const mode of ['general','learning','meeting']) {
    scene=mode;configuredKey=false;await act(async()=>{tree=create(React.createElement(load('app/interview/page.tsx').default));});
    await check('已获允许使用转写',true);assert.equal(button('开始持续转写').props.disabled,false);
    const before=calls.filter(c=>c.name==='answer_start').length;
    await click('开始持续转写');await scoped(9100,'这是一段用于测试离线记录的讨论');await latestTick();
    assert.equal(calls.filter(c=>c.name==='answer_start').length,before);
    await click('暂停');await click('恢复');await scoped(9101,'恢复后继续记录而不调用模型');await latestTick();
    assert.equal(calls.filter(c=>c.name==='answer_start').length,before,'resume silently enabled AI');
    if(mode==='meeting') {
      const fault={session_id:healthSnapshot.session_id,sequence:1,code:'capture_error'};
      healthSnapshot={...healthSnapshot,fault,system_frame_age_ms:null};
      await act(async()=>listeners.get('interview-audio-fault')({payload:fault}));
      healthSnapshot={...healthSnapshot,system_frame_age_ms:0};await act(async()=>timers.filter(t=>t.delay===500).at(-1).callback());
      await click('确认声音恢复，继续转写');await scoped(9102,'无 Key 的本地记录可在故障后恢复');await latestTick();
      assert.equal(answers().length,before);assert.ok(text(tree.toJSON()).includes('无 Key 的本地记录可在故障后恢复'));
    }
    await click('停止采集');await act(async()=>tree.unmount());
  }
  // Meeting summary is explicit and uses actual speech context; learning preserves AI context.
  for(const mode of ['learning','meeting']) {
    scene=mode;configuredKey=true;await act(async()=>{tree=create(React.createElement(load('app/interview/page.tsx').default));});
    await check('已获允许使用转写',true);await check('允许向 DeepSeek',true);await click('开始持续转写');
    const before=answers().length;await scoped(9200,'先讨论参与者的需求，再整理待确认事项');await latestTick();assert.equal(answers().length,before);
    await click(mode==='meeting'?'整理最近讨论':'解释最近内容');assert.equal(answers().length,before+1);
    assert.ok(answers().at(-1).args.dialogue.some(t=>t.text.includes('参与者的需求')));
    const first=answers().at(-1);await act(async()=>listeners.get('interview-answer')({payload:{question_id:first.args.questionId,revision:first.args.revision,event:{type:'delta',text:'建议回答：第一步澄清需求，第二步整理约束。'}}}));await finish();
    await act(async()=>tree.root.findByProps({id:'question'}).props.onChange({target:{value:'请展开上一轮解释'}}));await click('发送');
    assert.equal(answers().at(-1).args.history.at(-1).suggested_answer,'建议回答：第一步澄清需求，第二步整理约束。');await finish();
    if(mode==='learning') {
      const beforeSelf=answers().length;
      await emit({...own(9300,native.epoch,'请用一个日常例子解释临界点'),native_session_id:healthSnapshot.session_id});await latestTick();
      assert.equal(answers().length,beforeSelf);await click('用这句提问');
      assert.equal(tree.root.findByProps({id:'question'}).props.value,'请用一个日常例子解释临界点');assert.equal(answers().length,beforeSelf);
      await click('发送');assert.equal(answers().length,beforeSelf+1);await finish();
    }

    await click('停止采集');await act(async()=>tree.unmount());
  }
  scene='interview';configuredKey=true;
  await act(async()=>{tree=create(React.createElement(load('app/interview/page.tsx').default));});
  const choose=async(label,value)=>act(async()=>tree.root.findByProps({'aria-label':label}).props.onChange({target:{value}}));
  await choose('参会模式','groups');await choose('点名称呼','小林、Alex');
  await check('已获允许使用转写',true);await check('允许向 DeepSeek',true);await click('开始持续聆听与回答');
  const previous=answers().length;
  await scoped(9500,'其他同事在讨论预算');await latestTick();assert.equal(answers().length,previous);
  await scoped(9501,'小林。');await latestTick();assert.equal(answers().length,previous);
  assert(text(tree.toJSON()).includes('正在叫你 · 小林'));
  await scoped(9502,'你怎么看这次预算？');await latestTick();assert.equal(answers().length,previous+1);
  assert.equal(answers().at(-1).args.meetingMode,'groups');
  assert(answers().at(-1).args.dialogue.some(t=>t.text==='其他同事在讨论预算'));
  await finish();
  await choose('参会模式','pair');await scoped(9503,'下一题，不点名');await latestTick();assert.equal(answers().length,previous+2);
  await finish();await click('停止采集');await act(async()=>tree.unmount());
  console.log('PASS: actual page records group context, alerts before answering, dispatches only addressed speech, and switches back to pairs');
  console.log('PASS: all general scenes capture without Key/consent, resume stays local, and learning/meeting actions are explicit with labelled AI history');
  console.log('PASS: default/clear limit 80, restore counts, warning at 75, advance +20, in-flight completion, manual/auto block, continued capture/local self editing, retained queue and explicit resume');
  console.log('PASS: actual page fault/timer race, queue suppression, manual/local fallback, latched recovery, own speech, and old-session events');
  console.log('PASS: actual Stop/End/Clear controls pass native args; stop/start and clear/start work without remount; failed Trash keeps history');
  console.log('PASS: start with mic off, live off/on, stale epoch delivery blocked, new self speech retained, no self AI request');
  console.log('PASS: index clear is locked during session; Trash failure preserves index display/history; success preserves facts/history/call count; repeat empty clear handled');
})().catch(error=>{console.error(error);process.exitCode=1;});
