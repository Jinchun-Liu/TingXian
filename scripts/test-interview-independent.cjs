// Exercise the real audio panel, cold links, storage gate and session arbiter with a synthetic native adapter.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const base=path.resolve(__dirname,'../frontend'),req=require('node:module').createRequire(path.join(base,'package.json'));
const React=req('react'),{act,create}=req('react-test-renderer'),ts=req('typescript');
let prefs={preferred_mic_device:null,preferred_system_device:null},chosen='small',available=new Set(['small']),devices=[{name:'合成耳机',device_type:'Output'},{name:'合成麦克风',device_type:'Input'}],failBoot=true;
const active=new Map(),calls=[],routes=[];let releaseDownload, nativeDownload=null, cancelOutcome='cancelled';
const invoke=async(cmd,args)=>{
  calls.push(cmd);
  switch(cmd){
    case 'get_audio_devices':return devices;
    case 'get_recording_preferences':return {...prefs};
    case 'tingxian_storage_ready':if(failBoot)throw Error('合成只读失败');return;
    case 'tingxian_asr_models':return {selected:chosen,models:['small','base'].map(name=>({name,size_mb:100,status:nativeDownload?.name===name?{Downloading:{progress:nativeDownload.progress}}:available.has(name)?'Available':'Missing'}))};
    case 'tingxian_select_devices':prefs={preferred_mic_device:args.microphone,preferred_system_device:args.system};return;
    case 'tingxian_select_model':assert.ok(available.has(args.model));chosen=args.model;return;
    case 'whisper_download_model':await new Promise(r=>releaseDownload=r);available.add(args.modelName);return;
    case 'whisper_cancel_download':assert.equal(args.modelName,nativeDownload.name);return cancelOutcome;
    case 'trigger_system_audio_permission_command':return false;
    default:throw Error('unexpected IPC: '+cmd);
  }
};
const listen=async(name,fn)=>{active.set(name,fn);return()=>active.delete(name);};
global.window={addEventListener(){},removeEventListener(){},location:{search:'?id=a%2Fb%26c'}};
const router={replace(url){routes.push(url);}};
function component(rel){
  const src=fs.readFileSync(path.join(base,'src',rel),'utf8'),m={exports:{}};
  const code=ts.transpileModule(src,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX}}).outputText;
  new Function('require','module','exports',code)(n=>{
    if(n==='@tauri-apps/api/core')return {invoke,isTauri:()=>true};
    if(n==='@tauri-apps/api/event')return {listen};
    if(n==='next/navigation')return {useRouter:()=>router};
    if(n==='@/components/ui/button')return {Button:'button'};
    if(n==='@/components/ui/choice-menu')return {ChoiceMenu:'choice',ChoiceOption:'option'};
    return req(n);
  },m,m.exports);return m.exports;
}
(async()=>{
  const {AudioSettings}=component('components/interview/AudioSettings.tsx');let tree;
  const open=async()=>act(async()=>{tree=create(React.createElement(AudioSettings,{locked:false,microphone:false,onMicrophone(){}}));});
  const text=n=>typeof n==='string'?n:n.children.map(text).join('');
  const btn=s=>tree.root.findAllByType('button').find(n=>text(n)===s);
  const choose=async(label,value)=>act(async()=>{tree.root.findByProps({'aria-label':label}).props.onValueChange(value);});
  await open();assert(!calls.some(c=>/permission|download|start|save/.test(c)),'opening must not open audio or start a download');
  await choose('对方声音 · 系统输出','合成耳机 (output)');assert.equal(prefs.preferred_system_device,'合成耳机 (output)');
  await choose('本地 Whisper 模型','base');await act(async()=>{btn('下载模型').props.onClick();});
  assert.ok(btn('取消下载'));assert.equal(chosen,'small');await act(async()=>releaseDownload());await act(async()=>{btn('使用此模型').props.onClick();});assert.equal(chosen,'base');
  await act(async()=>{btn('测试系统音频').props.onClick();});assert(text(tree.root).includes('未能建立'));
  await act(async()=>tree.unmount());devices=[];await open();assert(text(tree.root).includes('未连接'));assert.equal(tree.root.findByProps({'aria-label':'本地 Whisper 模型'}).props.value,'base');
  await act(async()=>tree.unmount());
  // A native download continues when the settings tab is closed.
  nativeDownload={name:'small',progress:42};available.delete('small');
  await open();
  assert.ok(btn('取消下载'),'reopened settings must show the active download, even when another model is configured');
  assert(text(tree.root).includes('42%'));
  cancelOutcome='pending';
  await act(async()=>{await btn('取消下载').props.onClick();});
  assert(text(tree.root).includes('正在取消'));assert(!btn('下载模型'));
  nativeDownload=null;
  await act(async()=>{active.get('model-download-cancelled')({payload:{modelName:'small'}});});
  assert.ok(btn('下载模型'));assert.equal(chosen,'base');
  await act(async()=>tree.unmount());
  nativeDownload={name:'small',progress:27};await open();nativeDownload=null;
  await act(async()=>{active.get('model-download-error')({payload:{modelName:'small',error:'合成断线'}});});
  assert(text(tree.root).includes('合成断线'));assert.ok(btn('下载模型'));assert.equal(chosen,'base');
  await act(async()=>tree.unmount());
  const {StorageGate}=component('components/interview/StorageGate.tsx');
  await act(async()=>{tree=create(React.createElement(StorageGate,{},'就绪'));});assert.ok(btn('重新打开'));failBoot=false;
  await act(async()=>btn('重新打开').props.onClick());assert.equal(tree.toJSON(),'就绪');await act(async()=>tree.unmount());
  const {LegacyEntry}=component('components/interview/LegacyEntry.tsx');
  await act(async()=>{tree=create(React.createElement(LegacyEntry,{panel:'history'}));});assert.equal(new URL(routes[0],'https://local.invalid').searchParams.get('meeting'),'a/b&c');await act(async()=>tree.unmount());
  const {useSessionCommands}=component('lib/session-commands.ts');let send,finish;const routed=[];
  function Owner(){send=useSessionCommands(async cmd=>{routed.push(cmd);if(cmd==='start')await new Promise(r=>finish=r);});return null;}
  await act(async()=>{tree=create(React.createElement(Owner));});const job=send('start');
  active.get('tingxian-session-command')({payload:'start'});assert.deepEqual(routed,['start']);finish();await job;
  active.get('tingxian-session-command')({payload:'stop'});await Promise.resolve();assert.deepEqual(routed,['start','stop']);
  await act(async()=>tree.unmount());assert(!active.has('tingxian-session-command'));
  console.log('PASS: device and model survive remount, offline default, explicit download/use, permission failure, disconnected device, storage retry, legacy deep link, shared command exclusion');
})().catch(e=>{console.error(e);process.exitCode=1;});
