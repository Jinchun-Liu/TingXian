const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json')),ts=req('typescript');
const cache={};
function load(name){
 if(cache[name])return cache[name];
 const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/lib/'+name+'.ts'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText;
 const m={exports:{}};
 new Function('require','module','exports',code)(n=>n.startsWith('@/lib/')?load(n.slice(6)):req(n),m,m.exports);return cache[name]=m.exports;
}
const {RecoveryWriter,parseRecoverySnapshot}=load('interview-recovery');
const {recoverSessionSnapshot}=load('interview-session');
const fixture=(question='尚未发送')=>({version:1,snapshot:{version:1,history:[],transcripts:[],spoken_dialogue:[],current:{question:'',answer:''},calls:0,usage:'未知'},composer:{question,confirmed_answer:'本人草稿'}});
const turn={id:'q1',role:'remote',question:'请写出测试',suggestion:'已流出的部分代码',confirmed_answer:null,status:'generating',time:'00:01',timing:'',sources:[]};
const deferred=()=>{let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return {resolve,reject,promise};};
const settle=()=>new Promise(resolve=>setImmediate(resolve));
(async()=>{
 let current=fixture(),writes=[],counter=0,gate=null,failed=false,active=0,maxActive=0;
 const native=require('./test-support/recovery-native.cjs')();
 const invoke=async(name,args)=>{
  if(name==='interview_recovery_write'){
   writes.push(structuredClone(args));active++;maxActive=Math.max(maxActive,active);
   try{if(gate)await gate.promise;if(failed)throw Error('disk full');return await native(name,args);}finally{active--;}
  }
  return native(name,args);
 };
 const first=new RecoveryWriter(invoke,()=>current,()=>`window-${++counter}`);
 await first.initialize();assert.equal(writes.length,0,'opening app saved an empty window over recovery');
 await first.flush();assert.equal(writes.length,1);assert.equal(writes[0].draft.composer.question,'尚未发送');
 await first.flush();assert.equal(writes.length,1,'unchanged snapshot repeatedly wrote to disk');
 current=fixture('');current.composer.confirmed_answer='';await first.flush();
 assert.equal(writes.length,2,'erasing an already checkpointed editor must persist');
 current=fixture();current.snapshot.history=[{...turn}];current.snapshot.calls=1;
 current.snapshot.usage_ledger={version:1,legacy_calls:0,requests:[{question_id:'q1',revision:0,accepted:true,attempts:1,total_tokens:null,state:'pending'}]};
 gate=deferred();const old=first.flush();await settle();
 current=structuredClone(current);current.snapshot.history[0].suggestion+=' + later delta';
 const latest=first.flush();gate.resolve();await Promise.all([old,latest]);gate=null;
 assert.equal(maxActive,1,'concurrent checkpoint writes');
 assert(writes.at(-1).draft.snapshot.history[0].suggestion.endsWith('later delta'),'final flush lost a late streamed delta');
 const second=new RecoveryWriter(invoke,()=>fixture('new window'),()=>`window-${++counter}`);
 await second.initialize();assert.equal(second.getSnapshot().drafts.length,1);
 const previousId=second.getSnapshot().drafts[0].id;
 const saved=await native('interview_recovery_load',{id:previousId});
 const restored=recoverSessionSnapshot(parseRecoverySnapshot(saved).snapshot,'restore',true);
 assert.equal(restored.history[0].status,'cancelled');assert.equal(restored.calls,1);
 assert.equal(restored.usage_ledger.requests[0].state,'cancelled');
 assert(restored.history[0].suggestion.endsWith('later delta'));assert.equal(saved.composer.confirmed_answer,'本人草稿');
 await second.flush();assert.equal(native.drafts.size,2,'new owner overwrote previous unread draft');
 failed=true;current.composer.question='disk failure preserves latest live input';await assert.rejects(first.flush(),/disk full/);
 const count=writes.length;first.tick();await settle();assert.equal(writes.length,count,'full disk caused an automatic retry loop');
 assert(first.getSnapshot().error.includes('disk full'));failed=false;await first.retry();assert.equal(first.getSnapshot().error,'');
 gate=deferred();current.composer.question='before clear';const outstanding=first.flush();await settle();
 const paused=first.pause();gate.resolve();await Promise.all([outstanding,paused]);gate=null;
 native.clear();current=fixture('');current.composer.confirmed_answer='';await first.resetAfterClear();first.resume();
 await first.flush();assert.equal(native.drafts.size,0,'clear immediately recreated old content');
 current=fixture('new interview');await first.flush();assert.equal(native.drafts.size,1);
 assert.equal(writes.at(-1).epoch,1);assert.notEqual(writes.at(-1).id,previousId);
 await assert.rejects(second.flush().then(()=>{current=fixture('ignored');return invoke('interview_recovery_write',{...writes[0],revision:99});}),/stale epoch/);
 assert.throws(()=>parseRecoverySnapshot({...fixture(),version:8}));
 assert.throws(()=>parseRecoverySnapshot({...fixture(),composer:{question:3,confirmed_answer:''}}));
 // An initial empty editor is not stored. It cannot overwrite any previous draft.
 const empty=new RecoveryWriter(invoke,()=>({...fixture(''),composer:{question:'',confirmed_answer:''}}),()=>`window-${++counter}`);
 const before=writes.length;await empty.flush();assert.equal(writes.length,before);
 console.log('PASS recovery: editor-only drafts, serialized/coalesced streaming checkpoints, remount preservation, cancelled restoration, usage, disk failure/retry, cleanup fencing, no capture/API commands');
})().catch(e=>{console.error(e);process.exitCode=1;});
