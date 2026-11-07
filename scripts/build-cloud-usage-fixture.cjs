// Production HTML with local synthetic IPC; never connects to ASR or app storage.
const fs=require('node:fs'),path=require('node:path');
const output=process.argv[2],build=process.argv[3];if(!output||!build||fs.existsSync(output))throw Error('Pass a new output directory and a production export');
function fixture(){
 let sequence=0;const callbacks=new Map();
 const base={version:1,history:[],transcripts:[],spoken_dialogue:[],current:{question:'',answer:''},calls:0,usage:'未知'};
 const cloud={...base,cloud_asr_usage:{version:1,tasks:[{task_id:'12345678-1234-1234-1234-123456789001',source:'System',state:'finished',reported_seconds:42,inconsistent:false},{task_id:'12345678-1234-1234-1234-123456789002',source:'Microphone',state:'running',reported_seconds:null,inconsistent:false}]}};
 const legacy={...base,history:[{id:'old',role:'remote',question:'如何区分 null 和零？',suggestion:'分别核对字段类型和业务含义。',confirmed_answer:null,status:'done',time:'10:08',timing:'',sources:[]}]};
 let saved=JSON.parse(sessionStorage.getItem('cloud-usage-fixture')||'null')||[{id:1,saved_at:'2026-11-08T02:00:00Z',turns:0,calls:0,snapshot:cloud},{id:2,saved_at:'2026-11-08T01:00:00Z',turns:1,calls:0,snapshot:legacy}];
 let drafts={};const preferences={scenario:'interview',role:'',organization:'',context:'',terminology:'',answer_language:'auto',transcription_language:'auto'};
 window.isTauri=true;
 window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},transformCallback(fn){callbacks.set(++sequence,fn);return sequence;},unregisterCallback(id){callbacks.delete(id);},invoke:async(cmd,args)=>{
  if(cmd==='plugin:event|listen')return ++sequence;if(cmd==='plugin:event|unlisten')return;
  if(cmd==='interview_recovery_list')return {epoch:0,drafts:Object.values(drafts).map(({draft,...meta})=>meta)};
  if(cmd==='interview_recovery_load')return structuredClone(drafts[args.id].draft);
  if(cmd==='interview_recovery_write'){drafts[args.id]={id:args.id,revision:args.revision,updated_at:new Date().toISOString(),turns:args.draft.snapshot.history.length,calls:args.draft.snapshot.calls,draft:args.draft};return;}
  if(cmd==='interview_status')return {preferences,root:'',suggested_root:'',aliases:'',facts:'',key_configured:false,workspace_ready:false,index_documents:0,index_chunks:0,index_version:'1',documents:[],session_active:false,cloud_asr_mode:'local'};
  if(cmd==='interview_preferences_get')return preferences;
  if(cmd==='interview_request_config_get')return {model:'synthetic',connect_timeout_secs:5,first_answer_warning_secs:8,total_timeout_secs:60};
  if(cmd==='interview_connection_status')return {check_id:null,status:'untested',model:null,started_at:null,finished_at:null,elapsed_ms:null,attempts:0,total_tokens:null,message:'尚未测试连接'};
  if(cmd==='interview_residue_status')return {active:false,residue:{recording_folders:0,unfinished_folders:0,staged_archives:0,database_meetings:0,saved_sessions:saved.length}};
  if(cmd==='interview_session_list')return {sessions:saved.map(({snapshot,...meta})=>meta),next_before:null};
  if(cmd==='interview_session_load')return structuredClone(saved.find(s=>s.id===args.id).snapshot);
  if(cmd==='interview_session_save'){const row={id:Math.max(...saved.map(s=>s.id))+1,saved_at:new Date().toISOString(),turns:args.snapshot.history.length,calls:args.snapshot.calls,snapshot:structuredClone(args.snapshot)};saved.unshift(row);sessionStorage.setItem('cloud-usage-fixture',JSON.stringify(saved));const {snapshot,...meta}=row;return meta;}
  if(cmd==='session_clear')return;
  if(cmd==='interview_memory_open')return null;
  if(cmd==='interview_window_mode_get')return false;if(cmd==='api_get_meetings')return [];
  if(cmd==='get_recording_state')return {is_recording:false,is_paused:false,is_active:false,recording_duration:0,active_duration:0};
  if(cmd==='interview_audio_level')return {system:0,microphone:0,microphone_state:{recording:false,enabled:false,epoch:0}};
  if(['profile_import_status','interview_exit_status'].includes(cmd))return null;
  if(['interview_presentation_update','tingxian_storage_ready'].includes(cmd))return;
  throw Error('Synthetic fixture blocked '+cmd);
 }};
 window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
}
fs.mkdirSync(output);
for(const item of fs.readdirSync(build)){
 if(['interview.html','interview-workspace.html'].includes(item))fs.writeFileSync(path.join(output,item),fs.readFileSync(path.join(build,item),'utf8').replace('<head>','<head><script>('+fixture.toString()+')()</script>'));
 else fs.symlinkSync(path.join(path.resolve(build),item),path.join(output,item));
}
console.log(output);
