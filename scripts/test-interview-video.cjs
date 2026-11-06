// Decision: can the real panel expose status without capture and isolate late
// inspection replies after Stop/remount? Stop at any unexpected native command.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=req('react'),{act,create}=req('react-test-renderer'),ts=req('typescript');
const pending=[],calls=[];let status={generation:0,phase:'off',reason:null,inspection:null},failRead=false;
const invoke=(command,args)=>{
  calls.push({command,args});
  if(command==='video_status')return failRead?Promise.reject(Error('synthetic read failure')):Promise.resolve({...status});
  assert(['video_inspect','video_stop'].includes(command),'unexpected native/media command: '+command);
  return new Promise((resolve,reject)=>pending.push({command,args,resolve,reject}));
};
const mod={exports:{}};
const source=fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/VideoSettings.tsx'),'utf8');
new Function('require','module','exports',ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX}}).outputText)(name=>{
  if(name==='@tauri-apps/api/core')return {invoke};
  if(name==='@/components/ui/button')return {Button:'button'};
  return req(name);
},mod,mod.exports);
const text=node=>typeof node==='string'?node:(node?.children||[]).map(text).join('');
const report={generation:1,phase:'unavailable',reason:'runtime_not_pinned',inspection:{api_exports_present:true,machine_registration_present:true,user_registration_conflict:false}};
(async()=>{
  let tree;
  const open=()=>act(async()=>{tree=create(React.createElement(mod.exports.VideoSettings));});
  const button=label=>tree.root.findAllByType('button').find(node=>text(node)===label);
  const click=label=>act(async()=>{const node=button(label);assert(node,`missing ${label}`);assert(!node.props.disabled);node.props.onClick();});
  const finish=async(command,value,reject=false)=>{const index=pending.findIndex(p=>p.command===command);assert(index>=0);const p=pending.splice(index,1)[0];await act(async()=>reject?p.reject(value):p.resolve(value));};
  await open();assert.deepEqual(calls.map(c=>c.command),['video_status']);
  for(const label of ['本地预览','开启视频输出','开启脸部替换'])assert(button(label).props.disabled,label+' must remain disabled');
  assert(tree.root.findByType('select').props.disabled);
  assert(tree.root.findByType('input').props.disabled);assert.equal(tree.root.findByType('input').props.checked,false);
  await click('检查组件');assert.deepEqual(pending[0].args,{expectedGeneration:0});
  await click('停止检查');assert(button('正在停止…').props.disabled);
  // A successful late inspection arrives before Stop confirms: still stopping.
  await finish('video_inspect',report);assert(button('正在停止…'));
  await finish('video_stop',{...status,generation:2});assert(button('检查组件'));assert(!text(tree.root).includes('检测到注册'));
  await click('检查组件');assert.deepEqual(pending[0].args,{expectedGeneration:2});
  await click('停止检查');await finish('video_stop',{...status,generation:4});
  // A late native rejection after Stop must not replace the settled state.
  await finish('video_inspect',Error('old failure'),true);assert(!text(tree.root).includes('检查未完成'));
  await click('检查组件');await finish('video_inspect',{...report,generation:5});
  assert(text(tree.root).includes('眼部校正组件尚未提供'));assert(text(tree.root).includes('检测到注册'));
  await click('检查组件');await finish('video_inspect',{...status,generation:6,phase:'fault',reason:'cleanup'});
  assert(button('检查组件').props.disabled,'unconfirmed cleanup must forbid another launch');
  await act(async()=>tree.unmount());status={...status,generation:7};await open();
  await click('检查组件');await act(async()=>tree.unmount());
  await open();await finish('video_inspect',{...report,generation:8});
  assert(!text(tree.root).includes('检测到注册'),'old mount wrote into a new panel');
  await click('检查组件');await finish('video_inspect',Error('synthetic dispatch failure'),true);
  assert(!button('检查组件'),'unknown native state cannot launch another inspection');
  await click('刷新状态');assert(button('检查组件'));
  await act(async()=>tree.unmount());failRead=true;await open();
  assert(text(tree.root).includes('无法读取视频状态'));assert(!button('检查组件'));
  failRead=false;await click('刷新状态');assert(button('检查组件'));
  await act(async()=>tree.unmount());
  assert.equal(pending.length,0);
  console.log('PASS: video panel read-only mount, disabled media/face controls, generation admission, stop before/after late report, stale rejection/remount isolation, cleanup fault, native-state refresh recovery');
})().catch(error=>{console.error(error);process.exitCode=1;});
