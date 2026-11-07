// Isolated production HTML + strict synthetic bridge. No real DB, microphone,
// screenshot, credentials or model endpoints. Opening this is not device QA.
const fs=require('node:fs'),path=require('node:path');
const [output,build]=process.argv.slice(2);if(!output||!build)throw Error('Pass a new output directory and compiled frontend/out');
function fixture(){
 let seq=0;const callbacks=new Map();
 const preferences={scenario:'interview',role:'',organization:'',context:'',terminology:'',answer_language:'auto',transcription_language:'auto'};
 const request={model:'synthetic-model',connect_timeout_secs:5,first_answer_warning_secs:8,total_timeout_secs:60,budget:{context_tokens:262144,max_output_tokens:8192,safety_tokens:2048}};
 const profile=id=>({id,name:id==='compaction'?'会话记忆':id,enabled:true,endpoint:'https://synthetic.invalid/chat/completions',protocol:'openai',request:structuredClone(request)});
 let config={enabled:false,min_events:8,cooldown_ms:60000,max_requests:8,session_units:200000,profile:profile('compaction')};
 window.isTauri=true;window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
 window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},transformCallback(fn){callbacks.set(++seq,fn);return seq;},unregisterCallback(id){callbacks.delete(id);},invoke:async(cmd,args)=>{
  if(cmd==='plugin:event|listen')return ++seq;if(cmd==='plugin:event|unlisten')return;
  if(cmd==='interview_memory_config_get')return {config:structuredClone(config),key_configured:false};
  if(cmd==='interview_memory_config_save'){if(args.key)throw Error('本地预览禁止填写凭证');config=structuredClone(args.config);return;}
  if(cmd==='interview_memory_open')return null;
  if(cmd==='interview_memory_status')return {active:false};
  if(cmd==='interview_providers_get')return {config:{answers:[1,2,3].map(n=>profile('answer-'+n)),vision:profile('vision'),background:profile('background')},capacity:0,key_configured:{}};
  if(cmd==='interview_status')return {preferences,root:'',suggested_root:'',aliases:'',facts:'',key_configured:false,workspace_ready:false,index_documents:0,index_chunks:0,index_version:'1',documents:[],session_active:false,cloud_asr_mode:'local'};
  if(cmd==='interview_aliases_state')return {aliases:'',blocked_lines:[]};
  if(cmd==='interview_preferences_get')return preferences;
  if(cmd==='interview_preferences_state')return {preferences,blocked_fields:[]};
  if(cmd==='interview_request_config_get')return request;
  if(cmd==='interview_connection_status')return {status:'untested',message:'尚未测试连接'};
  if(cmd==='interview_recovery_list')return {epoch:0,drafts:[]};
  if(cmd==='interview_recovery_write')return;
  if(cmd==='interview_session_list')return {sessions:[],next_before:null};
  if(cmd==='interview_window_mode_get')return false;
  if(cmd==='api_get_meetings')return [];
  if(cmd==='get_recording_state')return {is_recording:false,is_paused:false,is_active:false,recording_duration:0,active_duration:0};
  if(cmd==='interview_audio_level')return {system:0,microphone:0,microphone_state:{recording:false,enabled:false,epoch:0}};
  if(cmd==='interview_residue_status')return {active:false,residue:{recording_folders:0,unfinished_folders:0,staged_archives:0,database_meetings:0,saved_sessions:0}};
  if(['profile_import_status','interview_exit_status'].includes(cmd))return null;
  if(['interview_presentation_update','interview_presentation_publish','tingxian_storage_ready','answer_cancel'].includes(cmd))return;
  throw Error('合成预览未提供此操作：'+cmd);
 }};
}
fs.mkdirSync(output,{recursive:true});for(const item of fs.readdirSync(build)){const target=path.join(output,item);if(['interview.html','interview-workspace.html'].includes(item))fs.writeFileSync(target,fs.readFileSync(path.join(build,item),'utf8').replace('<head>','<head><script>('+fixture.toString()+')()</script>'));else if(!fs.existsSync(target))fs.symlinkSync(path.resolve(build,item),target);}
console.log(output);
