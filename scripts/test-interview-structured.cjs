const recoveryNative=require('./test-support/recovery-native.cjs')();
// Real interview page: explicit JSON/YAML scopes, typed preview and stale reply isolation.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const requireFrontend=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=requireFrontend('react'),{create,act}=requireFrontend('react-test-renderer'),ts=requireFrontend('typescript');
const calls=[],listeners=new Map(),timers=[],native={recording:false};
let reply=null,delayed=false,fail=false,root='/synthetic';
const preview={root:'/synthetic',path:'facts.json',file_hash:'hash-v1',fields:[
 {pointer:'',kind:'object',preview:'',characters:0,children:2},
 {pointer:'/profile',kind:'object',preview:'',characters:0,children:3},
 {pointer:'/profile/null',kind:'null',preview:'null',characters:4,children:0},
 {pointer:'/profile/zero',kind:'number',preview:'0',characters:1,children:0},
 {pointer:'/profile/empty',kind:'string',preview:'""',characters:2,children:0},
 {pointer:'/excluded',kind:'string',preview:'excluded fixture',characters:16,children:0}
]};
const invoke=async(name,args)=>{
 calls.push({name,args});
    if(name.startsWith('interview_memory_'))return require('./test-support/memory-disabled.cjs')(name);
  if(name.startsWith('interview_recovery_'))return recoveryNative(name,args);
 if(name==='interview_status')return {root,suggested_root:'',aliases:'',facts:'',key_configured:true,index_version:'1',session_active:false};
 if(name==='profile_import_status')return null;
 if(name==='profile_scan')return [{path:'facts.json',bytes:150,eligible:true,reason:'需预览并选择字段或整份文件'}];
 if(name==='profile_structured_preview'){
  assert.equal(args.path,'facts.json');
  if(fail)throw Error('JSON 含重复字段名，请先消除歧义');
  if(delayed)return new Promise(resolve=>reply=resolve);
  return structuredClone(preview);
 }
 if(name==='profile_import')return;
 if(name==='interview_audio_level')return {system:0,microphone:0,microphone_state:{recording:false,enabled:false,epoch:0}};
 throw Error('unexpected '+name);
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
    if (name === '@/contexts/RecordingStateContext') return {useRecordingState:()=>({isRecording:native.recording,isActive:native.recording,isPaused:false,isStopping:false})};
    if (name === '@/contexts/TranscriptContext') return {useTranscripts:()=>({resetAfterHistoryClear:()=>{}})};
    if (name === '@/services/indexedDBService') return {indexedDBService:{suspendRecordingWrites(){},snapshotInterviewHistory:async()=>({meetings:[],transcripts:[]}),removeArchivedInterviewHistory:async()=>{}}};
    if (name === '@/components/ui/choice-menu') return {ChoiceMenu:'choice-menu',ChoiceOption:'choice-option'};
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
 const button=label=>tree.root.findAllByType('button').find(n=>text(n)===label);
 const click=async label=>{const b=button(label);assert.ok(b,label);assert.equal(!!b.props.disabled,false,label+' disabled');await act(async()=>await b.props.onClick());};
 const checkbox=label=>tree.root.findByProps({'aria-label':label});
 const check=async(label,on)=>{const n=checkbox(label);assert.equal(!!n.props.disabled,false);await act(async()=>n.props.onChange({target:{checked:on}}));};
 await click('资料与设置');await click('扫描预览');
 assert.equal(calls.filter(c=>c.name==='profile_structured_preview').length,0,'scanning must not parse structured files');
 await check('纳入 facts.json',true);
 assert.equal(button('导入 / 更新所选资料').props.disabled,true);
 await act(async()=>await button('导入 / 更新所选资料').props.onClick());
 assert.equal(calls.filter(c=>c.name==='profile_import').length,0);
 await click('预览字段');
 assert.equal(button('导入 / 更新所选资料').props.disabled,true,'preview is not selection');
 assert.ok(text(tree.toJSON()).includes('null 空值'));assert.ok(text(tree.toJSON()).includes('数字'));assert.ok(text(tree.toJSON()).includes('字符串'));
 await check('字段 facts.json /profile',true);
 assert.equal(button('导入 / 更新所选资料').props.disabled,false);
 assert.equal(checkbox('字段 facts.json /profile/null').props.checked,true);
 assert.equal(checkbox('字段 facts.json /profile/null').props.disabled,true,'selected parent includes child');
 assert.equal(checkbox('字段 facts.json /excluded').props.checked,false);
 await check('字段 facts.json /profile',false);
 await check('字段 facts.json /profile/null',true);await check('字段 facts.json /profile/zero',true);
 await check('整份文件 facts.json',true);assert.equal(checkbox('字段 facts.json /excluded').props.checked,true);
 await check('整份文件 facts.json',false);assert.equal(button('导入 / 更新所选资料').props.disabled,true);
 await check('字段 facts.json /profile/empty',true);
 fail=true;await click('重新预览字段');assert.ok(text(tree.toJSON()).includes('重复字段名'));
 assert.equal(button('导入 / 更新所选资料').props.disabled,true,'failed preview must invalidate old scope');
 fail=false;delayed=true;
 await act(async()=>{void button('预览字段').props.onClick();});
 assert.equal(button('扫描预览').props.disabled,true);
 await click('资料与设置'); // unmount while preview is still in flight
 await act(async()=>{reply(structuredClone(preview));});
 await click('资料与设置');assert.ok(button('预览字段'));assert.equal(button('导入 / 更新所选资料').props.disabled,true);
 assert.ok(!text(tree.toJSON()).includes('excluded fixture'),'late reply cannot populate a new panel');
 delayed=false;const originalFields=preview.fields;
 preview.fields=[{pointer:'',kind:'null',preview:'null',characters:4,children:0}];
 await click('预览字段');assert.ok(text(tree.toJSON()).includes('根值 · null 空值：null'));
 await check('整份文件 facts.json',true);assert.equal(button('导入 / 更新所选资料').props.disabled,false);
 preview.fields=originalFields;await click('重新预览字段');await check('字段 facts.json /profile/zero',true);
 await click('导入 / 更新所选资料');
 const sent=calls.filter(c=>c.name==='profile_import');assert.equal(sent.length,1);
 assert.deepEqual(sent[0].args.selected[0].structured,{root:'/synthetic',file_hash:'hash-v1',pointers:['/profile/zero']});
 assert.equal(calls.filter(c=>c.name==='interview_audio_start'||c.name==='answer_start').length,0);
 await act(async()=>tree.unmount());
 console.log('PASS: real page structured selection: no scan-time parsing, no implicit import, typed preview, subtree and whole-file selection, failed refresh invalidation, late reply isolation, exact native scope, no capture/API');
})().catch(e=>{console.error(e);process.exitCode=1;});
