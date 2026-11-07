// Production preview component with controlled native IPC. No screen/API access.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=req('react'),{create,act}=req('react-test-renderer'),ts=req('typescript');
const source=fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/ScreenshotControl.tsx'),'utf8');
const code=ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX}}).outputText;
const text=n=>typeof n==='string'?n:(n?.children||[]).map(text).join('');
const deferred=()=>{let resolve;return {promise:new Promise(r=>resolve=r),resolve:v=>resolve(v)};};
async function fixture(saved={}){
  const calls=[],sent=[],notices=[],storage=new Map(Object.entries(saved)),ref=React.createRef();
  let screens=[{id:7,width:1920,height:1080,primary:true}],gate=null,fail=false,reconfirm=false,count=0,tree;
  global.localStorage={getItem:k=>storage.get(k)??null,setItem:(k,v)=>storage.set(k,String(v)),removeItem:k=>storage.delete(k)};
  const invoke=async(cmd,args)=>{
    calls.push({cmd,args:structuredClone(args)});
    if(cmd==='interview_screenshot_displays')return structuredClone(screens);
    if(cmd==='interview_screenshot_preview_discard')return;
    if(cmd==='interview_screenshot_preview_create'){
      if(gate)await gate.promise;if(fail)throw Error('synthetic permission denied');
      return {preview_id:'asset-'+args.context.questionId,data_url:'data:image/png;base64,c3ludGhldGlj',display_id:args.displayId,topology:'synthetic-topology',expires_in_seconds:300,requires_confirmation:reconfirm};
    }
    throw Error(cmd);
  };
  const mod={exports:{}};
  new Function('require','module','exports',code)(name=>{
    if(name==='@tauri-apps/api/core')return {invoke};
    if(name==='@/components/ui/button')return {Button:'button'};
    if(name==='@/components/ui/choice-menu')return {ChoiceMenu:'select',ChoiceOption:'option'};
    if(name==='@/components/ui/dialog')return {Dialog:p=>p.open?React.createElement('dialog',p):null,DialogContent:'section',DialogTitle:'h2',DialogDescription:'p'};
    if(name==='lucide-react')return {Camera:()=>null};return req(name);
  },mod,mod.exports);
  let props={ref,disabled:false,canSend:true,contextKey:'original',makeContext:()=>({questionId:'q'+(++count),question:'区分 null 和零',history:[],dialogue:[{source:'Microphone',text:'我会检查幂等键'}],meetingMode:'pair'}),onSend:async c=>{sent.push(structuredClone(c));return true;},onNotice:m=>notices.push(m)};
  const render=()=>{const el=React.createElement(mod.exports.ScreenshotControl,props);tree?tree.update(el):tree=create(el);};
  await act(async()=>render());
  const button=label=>tree.root.findAllByType('button').find(n=>text(n)===label);
  const click=async label=>{const b=button(label);assert(b,`missing ${label}`);assert(!b.props.disabled,`disabled ${label}`);await act(async()=>{b.props.onClick();await new Promise(r=>setImmediate(r));});};
  return {calls,sent,notices,storage,ref,click,button,get tree(){return tree;},setScreens:v=>screens=v,setGate:v=>gate=v,setFail:v=>fail=v,setReconfirm:v=>reconfirm=v,
    update:async patch=>{props={...props,...patch};await act(async()=>render());},
    quick:async checked=>{await act(async()=>tree.root.findByType('input').props.onChange({target:{checked}}));},
    close:async()=>{await act(async()=>tree.unmount());}};
}
(async()=>{
  // Default and persisted quick mode still require first-session preview.
  for(const saved of [{},{'tingxian-screenshot-quick':'on'}]){
    const f=await fixture(saved);await f.click('截屏答题');assert.equal(f.calls.filter(c=>c.cmd.includes('create')).length,0);assert(f.button('发送并回答').props.disabled);
    await f.click('截取预览');assert.equal(f.sent.length,0);assert.equal(f.tree.root.findAllByType('img').length,1);
    await f.click('发送并回答');assert.equal(f.sent.length,1);assert.equal(f.sent[0].context.dialogue[0].text,'我会检查幂等键');
    assert(![...f.storage.values()].some(v=>v.includes('data:image')));await f.close();
  }
  // Explicit quick mode skips only later unchanged-target confirmations.
  const fast=await fixture();await fast.click('截屏答题');await fast.click('截取预览');await fast.quick(true);await fast.click('发送并回答');
  await fast.click('截屏答题');assert.equal(fast.sent.length,2);
  assert.notEqual(fast.sent[0].context.questionId,fast.sent[1].context.questionId);
  fast.setReconfirm(true);await fast.click('截屏答题');assert.equal(fast.sent.length,2);assert.equal(fast.tree.root.findAllByType('img').length,1);await fast.click('取消');
  fast.setScreens([{id:8,width:1280,height:720,primary:true}]);await fast.click('截屏答题');assert.equal(fast.sent.length,2);assert.equal(fast.tree.root.findAllByType('img').length,0);
  await fast.click('取消');await fast.close();
  // Quick mode always has a deliberate preview path to change target or opt out.
  const optout=await fixture();await optout.click('截屏答题');await optout.click('截取预览');await optout.quick(true);await optout.click('发送并回答');
  await optout.click('预览');assert.equal(optout.sent.length,1);assert.equal(optout.tree.root.findAllByType('img').length,0);
  await optout.quick(false);await optout.click('取消');await optout.click('截屏答题');assert.equal(optout.sent.length,1);
  assert.equal(optout.storage.get('tingxian-screenshot-quick'),'off');await optout.close();
  // Cancel during capture, then a late response, never submits or reopens.
  const race=await fixture({'tingxian-screenshot-quick':'on'});await race.click('截屏答题');const gate=deferred();race.setGate(gate);
  await race.click('截取预览');await race.click('取消');await act(async()=>{gate.resolve();await new Promise(r=>setImmediate(r));});
  assert.equal(race.sent.length,0);assert.equal(race.tree.root.findAllByType('img').length,0);assert(race.calls.some(c=>c.cmd.endsWith('_discard')));await race.close();
  // Question/session changes discard the old question-bound image.
  const changed=await fixture();await changed.click('截屏答题');await changed.click('截取预览');await changed.update({contextKey:'new-question'});
  assert.equal(changed.sent.length,0);assert.equal(changed.tree.root.findAllByType('img').length,0);assert(changed.notices.some(n=>n.includes('问题')));await changed.close();
  // Permission failure, revoked send permission, and busy lane preserve no hidden send.
  const denied=await fixture();await denied.click('截屏答题');denied.setFail(true);await denied.click('截取预览');assert.equal(denied.sent.length,0);assert(denied.tree.root.findByProps({role:'alert'}));await denied.close();
  const blocked=await fixture();await blocked.click('截屏答题');await blocked.click('截取预览');await blocked.update({canSend:false});assert(blocked.button('发送并回答').props.disabled);assert.equal(blocked.sent.length,0);
  await blocked.update({canSend:true,onSend:async()=>false});await blocked.click('发送并回答');assert.equal(blocked.tree.root.findAllByType('img').length,1);await blocked.close();
  // Global cancel / unmount destroys pending assets without creating an AI turn.
  const cancelled=await fixture();await cancelled.click('截屏答题');await cancelled.click('截取预览');await act(async()=>cancelled.ref.current.invalidate());
  assert.equal(cancelled.sent.length,0);assert(cancelled.calls.some(c=>c.cmd.endsWith('_discard')));await cancelled.close();
  console.log('PASS: preview-first, explicit quick mode, target changes, exact context, late capture, withdrawal, failure and cancellation (synthetic IPC only)');
})().catch(e=>{console.error(e);process.exitCode=1;});
