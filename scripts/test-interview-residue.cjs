// Real residue component, deferred native/cache responses: an old startup check
// must never expose a cleanup button over a newly active conversation.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const requireFrontend = require('node:module').createRequire(path.resolve(__dirname, '../frontend/package.json'));
const React = requireFrontend('react');
const {create, act} = requireFrontend('react-test-renderer');
const ts = requireFrontend('typescript');
const pendingNative = [], pendingCache = [];
let tauri = true, clears = 0;
const deferred = queue => new Promise((resolve,reject) => queue.push({resolve,reject}));
const moduleUnderTest = {exports:{}};
const compiled = ts.transpileModule(fs.readFileSync(path.resolve(__dirname, '../frontend/src/components/interview/ResidueNotice.tsx'), 'utf8'), {
  compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX}
}).outputText;
new Function('require','module','exports',compiled)(name => {
  if (name === '@tauri-apps/api/core') return {isTauri:()=>tauri, invoke:command=>{
    assert.equal(command,'interview_residue_status','inspection must never clean or send a request');
    return deferred(pendingNative);
  }};
  if (name === '@/services/indexedDBService') return {indexedDBService:{inspectInterviewHistory:()=>deferred(pendingCache)}};
  if (name === '@/components/ui/button') return {Button:props=>React.createElement('button',props)};
  return requireFrontend(name);
}, moduleUnderTest, moduleUnderTest.exports);
const {ResidueNotice} = moduleUnderTest.exports;
const nativeResult = (count=1) => ({active:false,residue:{recording_folders:count,unfinished_folders:count,staged_archives:0,database_meetings:count,saved_sessions:2}});
const text = node => typeof node==='string' ? node : (node?.children||[]).map(text).join('');
(async()=>{
  let tree, revision=0;
  const element = blocked => React.createElement(ResidueNotice,{blocked,revision,onClear:()=>clears++});
  const update = async blocked => {await act(async()=>tree.update(element(blocked)));};
  const resolve = async (queue,value) => {assert.ok(queue.length,'missing pending response');await act(async()=>queue.shift().resolve(value));};
  const button = label => tree.root.findAllByType('button').find(n=>text(n)===label);
  const click = async label => {assert.ok(button(label),label);await act(async()=>button(label).props.onClick());};
  await act(async()=>{tree=create(element(false));});
  assert.equal(pendingNative.length,1);
  await update(true);
  await resolve(pendingNative,nativeResult());
  assert.equal(pendingCache.length,0,'blocked late native result read cache');
  assert.equal(tree.toJSON(),null);

  await update(false);
  await resolve(pendingNative,nativeResult());
  assert.equal(pendingCache.length,1);
  await update(true);
  await resolve(pendingCache,{meetings:1,transcripts:3});
  assert.equal(tree.toJSON(),null,'late cache result exposed cleanup during capture');

  await update(false);
  await resolve(pendingNative,{active:true});
  assert.equal(pendingCache.length,0);assert.equal(tree.toJSON(),null);
  revision++;await update(false);
  await resolve(pendingNative,nativeResult(0));
  await resolve(pendingCache,{meetings:0,transcripts:0});
  assert.equal(tree.toJSON(),null,'saved sessions alone must not be treated as residue');

  revision++;await update(false);
  await act(async()=>pendingNative.shift().reject(Error('synthetic failed inspection')));
  assert.ok(text(tree.toJSON()).includes('记录保持原样'));
  assert.equal(button('清理上次残留，保留已保存会话'),undefined);
  await click('重新检查残留');
  await resolve(pendingNative,nativeResult());
  await resolve(pendingCache,{meetings:1,transcripts:0});
  assert.ok(text(tree.toJSON()).includes('1 个缓存会话（0 条转写）'));
  assert.ok(text(tree.toJSON()).includes('2 份明确保存的会话'));
  assert.equal(clears,0,'checks must not clean automatically');
  await click('暂时保留这些记录');
  assert.equal(button('清理上次残留，保留已保存会话'),undefined);
  await click('重新检查上次残留');
  await resolve(pendingNative,nativeResult());
  await resolve(pendingCache,{meetings:1,transcripts:2});
  await click('清理上次残留，保留已保存会话');assert.equal(clears,1);

  revision++;await update(false);
  await act(async()=>tree.unmount());
  await resolve(pendingNative,nativeResult());
  assert.equal(pendingCache.length,0,'unmounted check continued reading cache');
  tauri=false;
  await act(async()=>{tree=create(element(false));});
  assert.equal(pendingNative.length,0,'browser preview must not call native commands');
  await act(async()=>tree.unmount());
  console.log('PASS: late native/cache results suppressed on capture/unmount; active sessions and saved-only state hidden; failure, defer/recheck, explicit cleanup; no native calls in preview');
})().catch(error=>{console.error(error);process.exitCode=1;});
