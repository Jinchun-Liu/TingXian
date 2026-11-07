// Production page with synthetic IPC only. Never connects to user data or APIs.
const fs=require('node:fs'),path=require('node:path');
const output=process.argv[2];if(!output)throw Error('Pass a new output directory');
const buildFlag=process.argv.indexOf('--build');
const build=buildFlag<0?path.resolve(__dirname,'../frontend/out'):path.resolve(process.argv[buildFlag+1]);
function fixture(versions,transport){
 let sequence=0,recording=false,paused=false;const callbacks=new Map(),events=new Map();
 const emit=name=>{for(const cb of events.get(name)||[])cb({event:name,id:1,payload:null});};
 const preferences={scenario:'interview',role:'手动填写的测试岗位',organization:'原目标组织',context:'保留本人手动确认的背景。',terminology:'JSON Schema',answer_language:'auto',transcription_language:'auto'};
 const suggestions={role:'云产品测试实习生',organization:'示例组织',context:'仅完成本地模拟。接口自动化是否已完成存在冲突，生产回归尚未实施，需本人核实。',terminology:'Orchid, pytest, null, 契约测试',call_names:'林晓'};
 const profile={overview:'虚构验收资料：保留练习限制、来源和完成状态冲突。',role:suggestions.role,aliases:'契约测试,contract test'};
 const source='a'.repeat(64),expected='c'.repeat(64);
 const oldRow={location:'行 1',kind:'personal',quote:'Orchid 生产经验：无。仅完成本地模拟。',note:'旧版来源，尚未完成生产回归。'};
 const newRow={location:'行 1',kind:'personal',quote:'Orchid 生产经验：有。已参与生产回归。',note:'新版说法不同，需本人核对职责与完成时间。'};
 const documents=versions?[{path:'.背景/'+source+'/项目说明.md',root:'背景资料库',hash:expected,kind:'personal',allow_external:false,chunks:1,metadata:{confirmation:'content_changed',validity:'content_changed',confirmed_at:null,valid_from:null,valid_until:null,updated_at:null}}]:[];
 const provider=id=>({id,name:'合成 '+id,enabled:id==='answer-1',endpoint:'https://synthetic.invalid/chat/completions',protocol:'openai',request:{model:'synthetic-model',connect_timeout_secs:5,first_answer_warning_secs:8,total_timeout_secs:20}});
 let providers={answers:[1,2,3].map(i=>provider('answer-'+i)),vision:provider('vision'),background:provider('background')};
 window.isTauri=true;
 window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},
  transformCallback(fn){callbacks.set(++sequence,fn);return sequence;},unregisterCallback(id){callbacks.delete(id);},
  invoke:async(cmd,args)=>{
   if(cmd==='plugin:event|listen'){events.set(args.event,[...(events.get(args.event)||[]),callbacks.get(args.handler)]);return ++sequence;}
   if(cmd==='plugin:event|unlisten')return;
   if(cmd==='interview_status')return {preferences,root:'',suggested_root:'',aliases:'',facts:'',key_configured:transport,workspace_ready:transport,answer_capacity:transport?2:0,cloud_asr_mode:'local',index_documents:1,index_chunks:4,index_version:'8',documents,session_active:false,architecture:'合成测试',system_version:'未连接原生应用',memory_bytes:0,whisper_model:''};
   if(cmd==='background_status')return {library:{documents:1,chunks:4,baseline:262144,full_budget:62000,rag_budget:56000,fits_full:true,version:'8',state:{profile,share:false,metrics:null},measurement:null},model:'合成模型',ready:false,jobs:[],work:{busy:false,progress:'',draft:versions?{id:'draft-versions',profile,documents:[{id:source,name:'项目说明.md',count:1}],failures:[],metrics:null}:null}};
   if(cmd==='background_changes')return [{id:source,name:'项目说明.md',state:'source_changed',identical_count:0,identical_sources:[]}];
   if(cmd==='background_preview')return {rows:[newRow],more:false,previous:{name:'项目说明.md',rows:[oldRow],more:false}};
   if(cmd==='background_versions')return {rows:[{revision:expected,name:'项目说明.md',source_hash:expected,recorded_at:'2026-11-06T01:00:00Z',count:1,current:true},{revision:'b'.repeat(64),name:'项目说明.md',source_hash:'b'.repeat(64),recorded_at:null,count:1,current:false}],more:false};
   if(cmd==='background_version_records')return {rows:[args.revision===expected?newRow:oldRow],current:args.revision===expected,more:false};
   if(cmd==='background_settings_preview')return {version:'8',profile_hash:'synthetic-hash',current:{...preferences},suggestions};
   if(cmd==='background_settings_apply'){
    if(args.version!=='8'||args.profileHash!=='synthetic-hash'||args.changes.some(c=>preferences[c.field]!==c.expected))throw Error('资料或设置已变化');
    for(const c of args.changes){if(!['role','organization','context','terminology'].includes(c.field))throw Error('unsupported field');preferences[c.field]=c.value;}return;
   }
   if(cmd==='interview_preferences_get')return preferences;
   if(cmd==='interview_providers_get')return {config:structuredClone(providers),key_configured:{},capacity:0};
   if(cmd==='interview_providers_save'){if(Object.keys(args.keys).length)throw Error('合成页面不接收凭证');providers=structuredClone(args.config);return;}
   if(cmd==='interview_provider_test')return {success:true,message:'合成固定文字结果，未连接真实模型',elapsed_ms:0,attempts:0,total_tokens:null};
   if(cmd==='interview_recovery_list')return {epoch:0,drafts:[]};
   if(cmd==='interview_request_config_get')return {model:'synthetic',connect_timeout_secs:5,first_answer_warning_secs:8,total_timeout_secs:60};
   if(cmd==='interview_connection_status')return {status:'untested',message:'尚未测试连接'};
   if(cmd==='interview_residue_status')return {active:false,residue:{recording_folders:0,unfinished_folders:0,staged_archives:0,database_meetings:0,saved_sessions:0}};
   if(cmd==='interview_session_list')return {sessions:[],next_before:null};
   if(cmd==='interview_window_mode_get')return false;
   if(cmd==='api_get_meetings')return [];
   if(cmd==='get_recording_state')return {is_recording:recording,is_paused:paused,is_active:recording&&!paused,recording_duration:0,active_duration:0};
   if(cmd==='interview_audio_level')return {system:0,microphone:0,microphone_state:{recording,enabled:false,epoch:0}};
   if(transport){
    if(cmd==='interview_audio_start'){recording=true;paused=false;emit('recording-started');return {recording,enabled:false,epoch:0};}
    if(cmd==='pause_recording'){paused=true;emit('recording-paused');return;}
    if(cmd==='resume_recording'){paused=false;emit('recording-resumed');return;}
    if(cmd==='stop_recording'){recording=false;paused=false;emit('recording-stopped');return;}
    if(cmd==='answer_cancel')return;
   }
   if(['profile_import_status','interview_exit_status'].includes(cmd))return null;
   if(['interview_presentation_update','interview_recovery_write','tingxian_storage_ready'].includes(cmd))return;
   throw Error('Synthetic fixture blocked '+cmd);
  }
 };
 window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
}
fs.mkdirSync(output); // Refuse reusing a fixture directory or modifying the build.
for(const item of fs.readdirSync(build)){
 if(item==='interview-workspace.html'||(process.argv.includes('--transport')&&item==='interview.html'))fs.writeFileSync(path.join(output,item),fs.readFileSync(path.join(build,item),'utf8').replace('<head>','<head><script>('+fixture.toString()+')('+JSON.stringify(process.argv.includes('--versions'))+','+JSON.stringify(process.argv.includes('--transport'))+')</script>'));
 else fs.symlinkSync(path.join(build,item),path.join(output,item));
}
console.log(output);
