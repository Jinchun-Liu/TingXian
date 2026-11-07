// Production workspace HTML and an explicit synthetic IPC bridge. No native capture.
const fs=require('node:fs'),path=require('node:path');
const [output,pngPath,buildPath]=process.argv.slice(2);if(!output||!pngPath)throw Error('Pass a new output directory and a synthetic PNG');
const build=buildPath?path.resolve(buildPath):path.resolve(__dirname,'../frontend/out'),png=fs.readFileSync(pngPath);
if(!png.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10])))throw Error('Expected PNG');
function fixture(image){
  let seq=0,previews=0,answers=0,screen=7;const callbacks=new Map(),events=new Map();
  localStorage.removeItem('tingxian-screenshot-quick');localStorage.removeItem('tingxian-screenshot-display');
  const show=()=>{const el=document.getElementById('fixture-count');if(el)el.textContent=`合成测试 · 本地预览 ${previews} · AI 请求 ${answers}`;};
  const emit=(name,payload)=>{for(const fn of events.get(name)||[])fn({event:name,id:1,payload});};
  const preferences={scenario:'interview',role:'',organization:'',context:'',terminology:'',answer_language:'auto',transcription_language:'auto'};
  window.isTauri=true;window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},transformCallback(fn){callbacks.set(++seq,fn);return seq;},unregisterCallback(id){callbacks.delete(id);},invoke:async(cmd,args)=>{
    if(cmd==='plugin:event|listen'){events.set(args.event,[...(events.get(args.event)||[]),callbacks.get(args.handler)]);return ++seq;}
    if(cmd==='plugin:event|unlisten')return;
    if(cmd==='interview_status')return {preferences,root:'',suggested_root:'',aliases:'',facts:'',key_configured:true,workspace_ready:true,answer_capacity:3,vision_ready:true,vision_model:'合成视觉模型',index_documents:0,index_chunks:0,index_version:'1',documents:[],session_active:false};
    if(cmd==='interview_recovery_list')return {epoch:0,drafts:[]};
    if(cmd==='interview_recovery_write')return;
    if(cmd==='interview_preferences_get')return preferences;
    if(cmd==='interview_request_config_get')return {model:'synthetic',connect_timeout_secs:5,first_answer_warning_secs:8,total_timeout_secs:60};
    if(cmd==='interview_connection_status')return {status:'untested',message:'未连接模型'};
    if(cmd==='interview_residue_status')return {active:false,residue:{recording_folders:0,unfinished_folders:0,staged_archives:0,database_meetings:0,saved_sessions:0}};
    if(cmd==='interview_session_list')return {sessions:[],next_before:null};
    if(cmd==='interview_window_mode_get')return false;
    if(cmd==='api_get_meetings')return [];
    if(cmd==='get_recording_state')return {is_recording:false,is_paused:false,is_active:false,recording_duration:0,active_duration:0};
    if(cmd==='interview_audio_level')return {system:0,microphone:0,microphone_state:{recording:false,enabled:false,epoch:0}};
    if(cmd==='interview_memory_open')return null;
    if(cmd==='interview_memory_status')return {enabled:false,active:false};
    if(['profile_import_status','interview_exit_status'].includes(cmd))return null;
    if(['interview_presentation_update','tingxian_storage_ready','answer_cancel'].includes(cmd))return;
    if(cmd==='interview_screenshot_displays')return [{id:screen,width:1920,height:1080,primary:true},{id:9,width:2560,height:1440,primary:false}];
    if(cmd==='interview_screenshot_preview_create'){previews++;show();return {preview_id:'fixture-'+previews,data_url:image,display_id:args.displayId,topology:'fixture-'+screen,expires_in_seconds:300,requires_confirmation:false};}
    if(cmd==='interview_screenshot_preview_discard')return;
    if(cmd==='interview_screenshot_preview_answer_start'){
      answers++;show();const meta={question_id:args.context.questionId,revision:args.revision};
      setTimeout(()=>{emit('interview-answer',{...meta,event:{type:'delta',text:'先检查图中条件，再分别验证 null、零和字段缺失。\n<<<QUESTION>>>\n请分析这道合成截图题。'}});emit('interview-answer',{...meta,event:{type:'done',elapsed_ms:300,usage:null}});},300);return;
    }
    throw Error('Synthetic fixture blocked '+cmd);
  }};
  window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
  window.addEventListener('load',()=>{const bar=document.createElement('aside');bar.style.cssText='position:fixed;right:12px;bottom:8px;z-index:20;padding:6px 12px;border:1px solid #C5C7CA;background:white;font-size:12px;border-radius:8px';const label=document.createElement('span');label.id='fixture-count';bar.appendChild(label);const b=document.createElement('button');b.textContent=' · 模拟屏幕变化';b.onclick=()=>{screen=screen===7?8:7;show();};bar.appendChild(b);document.body.appendChild(bar);show();});
}
fs.mkdirSync(output);
for(const name of fs.readdirSync(build)){
  if(name==='interview-workspace.html')fs.writeFileSync(path.join(output,name),fs.readFileSync(path.join(build,name),'utf8').replace('<head>','<head><script>('+fixture.toString()+')('+JSON.stringify('data:image/png;base64,'+png.toString('base64'))+')</script>'));
  else fs.symlinkSync(path.join(build,name),path.join(output,name));
}
console.log(output);
