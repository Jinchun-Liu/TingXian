// Production HTML + a synthetic, local-only IPC bridge. Never touches the app DB.
const fs=require('node:fs'),path=require('node:path');
const output=process.argv[2];if(!output)throw Error('Pass a new output directory');
const build=path.resolve(__dirname,'../frontend/out');
function fixture(){
  let sequence=0;const callbacks=new Map();
  const row={id:'q1',role:'remote',question:'如何区分 null、零与字段缺失？',suggestion:'先分别验证 JSON Schema 和业务语义。null、零和缺失字段应各有测试用例。',confirmed_answer:null,status:'generating',time:'10:08',timing:'',sources:[]};
  const seed={version:1,snapshot:{version:1,history:[row],transcripts:[],spoken_dialogue:[],current:{question:row.question,answer:row.suggestion},calls:1,usage:'用量未知'},composer:{question:'尚未发送的后续追问',confirmed_answer:'我会验证幂等键与租户隔离'}};
  let drafts=JSON.parse(sessionStorage.getItem('recovery-fixture')||'null')||{'interrupted-window':{id:'interrupted-window',revision:1,updated_at:'2026-11-06T02:08:00Z',turns:1,calls:1,draft:seed}};
  const preferences={scenario:'interview',role:'',organization:'',context:'',terminology:'',answer_language:'auto',transcription_language:'auto'};
  window.isTauri=true;
  window.__TAURI_INTERNALS__={
    metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},
    transformCallback(fn){callbacks.set(++sequence,fn);return sequence;},unregisterCallback(id){callbacks.delete(id);},
    invoke:async(cmd,args)=>{
      if(cmd==='plugin:event|listen')return ++sequence;
      if(cmd==='plugin:event|unlisten')return;
      if(cmd==='interview_recovery_list')return {epoch:0,drafts:Object.values(drafts).map(({draft,...meta})=>meta)};
      if(cmd==='interview_recovery_load')return structuredClone(drafts[args.id].draft);
      if(cmd==='interview_recovery_write'){drafts[args.id]={id:args.id,revision:args.revision,updated_at:new Date().toISOString(),turns:args.draft.snapshot.history.length,calls:args.draft.snapshot.calls,draft:args.draft};sessionStorage.setItem('recovery-fixture',JSON.stringify(drafts));return;}
      if(cmd==='interview_status')return {preferences,root:'',suggested_root:'',aliases:'',facts:'',key_configured:false,workspace_ready:false,index_documents:0,index_chunks:0,index_version:'1',documents:[],session_active:false};
      if(cmd==='interview_preferences_get')return preferences;
      if(cmd==='interview_request_config_get')return {model:'synthetic',connect_timeout_secs:5,first_answer_warning_secs:8,total_timeout_secs:60};
      if(cmd==='interview_connection_status')return {status:'untested',message:'尚未测试连接'};
      if(cmd==='interview_residue_status')return {active:false,residue:{recording_folders:0,unfinished_folders:0,staged_archives:0,database_meetings:0,saved_sessions:0}};
      if(cmd==='interview_session_list')return {sessions:[],next_before:null};
      if(cmd==='interview_window_mode_get')return false;
      if(cmd==='api_get_meetings')return [];
      if(cmd==='get_recording_state')return {is_recording:false,is_paused:false,is_active:false,recording_duration:0,active_duration:0};
      if(cmd==='interview_audio_level')return {system:0,microphone:0,microphone_state:{recording:false,enabled:false,epoch:0}};
      if(['profile_import_status','interview_exit_status'].includes(cmd))return null;
      if(['interview_presentation_update','tingxian_storage_ready'].includes(cmd))return;
      throw Error('Synthetic fixture blocked '+cmd);
    }
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
}
fs.mkdirSync(output,{recursive:true});
for(const item of fs.readdirSync(build)){
  if(['interview.html','interview-workspace.html'].includes(item)){
    fs.writeFileSync(path.join(output,item),fs.readFileSync(path.join(build,item),'utf8').replace('<head>','<head><script>('+fixture.toString()+')()</script>'));
  }else if(!fs.existsSync(path.join(output,item)))fs.symlinkSync(path.join(build,item),path.join(output,item));
}
console.log(output);
