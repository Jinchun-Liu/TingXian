// Actual connection panel, no external calls; exercises pending RPCs and stale results.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const frontend=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=frontend('react'),{act,create}=frontend('react-test-renderer'),ts=frontend('typescript');
const blank=()=>({check_id:null,status:'untested',model:null,started_at:null,finished_at:null,elapsed_ms:null,attempts:0,total_tokens:null,message:'尚未测试连接'});
let remote=blank(),pendingTest=null,pendingCancel=null,pendingRead=null,rejectStart=false;
let props={disabled:false,hasKey:false,revision:0},tree;const calls=[],checking=[],timers=new Map();let timer=0;
const result=(id,status,message)=>({...blank(),check_id:id,status,model:'synthetic-model',finished_at:new Date().toISOString(),elapsed_ms:12,attempts:1,total_tokens:9,message});
const invoke=async(name,args)=>{
 calls.push({name,args});
 if(name==='interview_connection_status'){if(pendingRead)return new Promise(resolve=>{pendingRead=resolve;});return {...remote};}
 if(name==='interview_connection_test'){
  assert.deepEqual(Object.keys(args).sort(),['checkId','confirmed']);assert.equal(args.confirmed,true);if(rejectStart)throw Error('synthetic preflight failure');
  remote={...blank(),check_id:args.checkId,status:'checking',model:'synthetic-model',message:'正在测试'};
  return new Promise(resolve=>{pendingTest={id:args.checkId,resolve};});
 }
 assert.equal(name,'interview_connection_cancel');assert.equal(args.checkId,remote.check_id);
 remote={...remote,status:'cancelling',message:'正在取消'};
 if(pendingCancel)return new Promise(resolve=>{pendingCancel=resolve;});
 return {...remote};
};
const moduleObject={exports:{}};
const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/ConnectionCheck.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX,esModuleInterop:true}}).outputText;
new Function('require','module','exports','setInterval','clearInterval',code)(name=>name==='@tauri-apps/api/core'?{invoke}:name==='@/components/ui/button'?{Button:p=>React.createElement('button',p)}:frontend(name),moduleObject,moduleObject.exports,fn=>{timers.set(++timer,fn);return timer;},id=>timers.delete(id));
const {ConnectionCheck}=moduleObject.exports,text=n=>typeof n==='string'?n:(n?.children||[]).map(text).join('');
const render=()=>{const e=React.createElement(ConnectionCheck,{...props,onCheckingChange:v=>checking.push(v)});if(tree)tree.update(e);else tree=create(e);};
const button=label=>tree.root.findAllByType('button').find(n=>text(n)===label);
const click=async label=>{const b=button(label);assert.ok(b);assert.ok(!b.props.disabled);await act(async()=>{void b.props.onClick();});};
const finish=async(status,message)=>act(async()=>{const p=pendingTest;pendingTest=null;remote=result(p.id,status,message);p.resolve({...remote});});
const countTests=()=>calls.filter(c=>c.name==='interview_connection_test').length;
(async()=>{
 await act(async()=>render());assert.equal(countTests(),0);assert.ok(button('测试 DeepSeek 连接').props.disabled);
 props.hasKey=true;await act(async()=>render());await click('测试 DeepSeek 连接');assert.equal(countTests(),1);assert.equal(checking.at(-1),true);assert.ok(button('刷新连接状态').props.disabled);
 await finish('passed','连接成功');assert.equal(checking.at(-1),false);assert.ok(text(tree.toJSON()).includes('上次测试成功'));assert.ok(text(tree.toJSON()).includes('9 tokens'));
 // A stale status read must not revive success after the configuration changed.
 pendingRead=true;await click('刷新连接状态');const stale=pendingRead;const old={...remote};remote=blank();props.revision++;
 await act(async()=>render());pendingRead=null;await act(async()=>stale(old));
 assert.ok(text(tree.toJSON()).includes('API 连接：尚未测试'));assert.equal(countTests(),1);
 await click('测试 DeepSeek 连接');await finish('failed','API Key 无效（401）');assert.ok(text(tree.toJSON()).includes('401'));assert.equal(checking.at(-1),false);
 await click('测试 DeepSeek 连接');pendingCancel=true;await click('取消连接测试');const lateCancel=pendingCancel;const ack={...remote};
 await finish('cancelled','已取消');pendingCancel=null;await act(async()=>lateCancel(ack));
 assert.ok(text(tree.toJSON()).includes('上次测试已取消'));assert.ok(!button('取消连接测试'),'late cancel response regressed finished status');assert.equal(checking.at(-1),false);
 rejectStart=true;await click('测试 DeepSeek 连接');assert.ok(text(tree.toJSON()).includes('synthetic preflight failure'));assert.equal(checking.at(-1),false);rejectStart=false;
 props.disabled=true;await act(async()=>render());assert.ok(button('测试 DeepSeek 连接').props.disabled);props.disabled=false;await act(async()=>render());
 await click('测试 DeepSeek 连接');const id=pendingTest.id;await act(async()=>tree.unmount());tree=null;
 assert.equal(calls.at(-1).name,'interview_connection_cancel');assert.equal(calls.at(-1).args.checkId,id);
 await finish('cancelled','离开页面已取消');assert.equal(timers.size,0);
 assert.ok(calls.every(c=>['interview_connection_status','interview_connection_test','interview_connection_cancel'].includes(c.name)));
 console.log('PASS: connection panel sends only explicit fixed-test IPC, keeps checks separate, shows usage/errors, invalidates stale reads, handles cancel races/preflight failures, locks in session and cancels on unmount');
})().catch(e=>{console.error(e);process.exitCode=1;});
