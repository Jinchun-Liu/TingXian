// Production page with synthetic IPC only. Never connects to user data or APIs.
const fs=require('node:fs'),path=require('node:path');
const output=process.argv[2];if(!output)throw Error('Pass a new output directory');
const build=path.resolve(__dirname,'../frontend/out');
function fixture(){
 let sequence=0;const callbacks=new Map();
 const preferences={scenario:'interview',role:'手动填写的测试岗位',organization:'原目标组织',context:'保留本人手动确认的背景。',terminology:'JSON Schema',answer_language:'auto',transcription_language:'auto'};
 const suggestions={role:'云产品测试实习生',organization:'示例组织',context:'仅完成本地模拟。接口自动化是否已完成存在冲突，生产回归尚未实施，需本人核实。',terminology:'Orchid, pytest, null, 契约测试',call_names:'林晓'};
 const profile={overview:'虚构验收资料：保留练习限制、来源和完成状态冲突。',role:suggestions.role,aliases:'契约测试,contract test'};
 window.isTauri=true;
 window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},
  transformCallback(fn){callbacks.set(++sequence,fn);return sequence;},unregisterCallback(id){callbacks.delete(id);},
  invoke:async(cmd,args)=>{
   if(cmd==='plugin:event|listen')return ++sequence;
   if(cmd==='plugin:event|unlisten')return;
   if(cmd==='interview_status')return {preferences,root:'',suggested_root:'',aliases:'',facts:'',key_configured:false,workspace_ready:false,index_documents:1,index_chunks:4,index_version:'8',documents:[],session_active:false,architecture:'合成测试',system_version:'未连接原生应用',memory_bytes:0,whisper_model:''};
   if(cmd==='background_status')return {library:{documents:1,chunks:4,baseline:262144,full_budget:62000,rag_budget:56000,fits_full:true,version:'8',state:{profile,share:false,metrics:null},measurement:null},model:'合成模型',ready:false,jobs:[],work:{busy:false,progress:'',draft:null}};
   if(cmd==='background_settings_preview')return {version:'8',profile_hash:'synthetic-hash',current:{...preferences},suggestions};
   if(cmd==='background_settings_apply'){
    if(args.version!=='8'||args.profileHash!=='synthetic-hash'||args.changes.some(c=>preferences[c.field]!==c.expected))throw Error('资料或设置已变化');
    for(const c of args.changes){if(!['role','organization','context','terminology'].includes(c.field))throw Error('unsupported field');preferences[c.field]=c.value;}return;
   }
   if(cmd==='interview_preferences_get')return preferences;
   if(cmd==='interview_recovery_list')return {epoch:0,drafts:[]};
   if(cmd==='interview_request_config_get')return {model:'synthetic',connect_timeout_secs:5,first_answer_warning_secs:8,total_timeout_secs:60};
   if(cmd==='interview_connection_status')return {status:'untested',message:'尚未测试连接'};
   if(cmd==='interview_residue_status')return {active:false,residue:{recording_folders:0,unfinished_folders:0,staged_archives:0,database_meetings:0,saved_sessions:0}};
   if(cmd==='interview_session_list')return {sessions:[],next_before:null};
   if(cmd==='interview_window_mode_get')return false;
   if(cmd==='api_get_meetings')return [];
   if(cmd==='get_recording_state')return {is_recording:false,is_paused:false,is_active:false,recording_duration:0,active_duration:0};
   if(cmd==='interview_audio_level')return {system:0,microphone:0,microphone_state:{recording:false,enabled:false,epoch:0}};
   if(['profile_import_status','interview_exit_status'].includes(cmd))return null;
   if(['interview_presentation_update','interview_recovery_write','tingxian_storage_ready'].includes(cmd))return;
   throw Error('Synthetic fixture blocked '+cmd);
  }
 };
 window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
}
fs.mkdirSync(output); // Refuse reusing a fixture directory or modifying the build.
for(const item of fs.readdirSync(build)){
 if(item==='interview-workspace.html')fs.writeFileSync(path.join(output,item),fs.readFileSync(path.join(build,item),'utf8').replace('<head>','<head><script>('+fixture.toString()+')()</script>'));
 else fs.symlinkSync(path.join(build,item),path.join(output,item));
}
console.log(output);
