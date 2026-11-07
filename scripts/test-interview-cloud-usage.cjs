// Observed task seconds and persistence; no service, credentials or capture.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json')),ts=req('typescript');
const cache={};function load(name){if(cache[name])return cache[name];const m={exports:{}};const src=fs.readFileSync(path.resolve(__dirname,'../frontend/src/lib',name+'.ts'),'utf8');
  new Function('require','module','exports',ts.transpileModule(src,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.CommonJS}}).outputText)(n=>n.startsWith('@/lib/')?load(n.slice(6)):n.startsWith('./')?load(n.slice(2)):req(n),m,m.exports);return cache[name]=m.exports;}
const {CloudUsage,parseCloudUsage,cloudUsageSummary,cloudUsageLabel}=load('interview-cloud-usage');
const {parseSessionSnapshot,recoverSessionSnapshot}=load('interview-session');
const id=n=>`12345678-1234-1234-1234-${String(n).padStart(12,'0')}`;
const row=(n,seconds,state='finished',source='System')=>({task_id:id(n),source,state,reported_seconds:seconds,inconsistent:false});
(async()=>{
 let responses={version:1,tasks:[]},hold=null,count=0,seq=100;const calls=[];
 const ledger=new CloudUsage(async(cmd,args)=>{calls.push({cmd,args});if(hold)await hold;return structuredClone(responses);},()=>id(seq++));
 assert.deepEqual(await ledger.flush(),{version:1,tasks:[]});assert.equal(calls.length,0);
 const scope=ledger.begin();responses.tasks=[row(1,3,'running')];await ledger.flush();responses.tasks[0].reported_seconds=8;await ledger.flush();await ledger.flush();
 assert.equal(cloudUsageSummary(ledger.snapshot()).seconds,'8');assert.equal(ledger.begin(),scope);
 responses.tasks.push(row(2,0,'cancelled','Microphone'),row(3,null,'failed'));await ledger.flush();
 assert.deepEqual(cloudUsageSummary(ledger.snapshot()),{attempts:3,known:2,unknown:1,unfinished:3,seconds:'8'});assert.match(cloudUsageLabel(ledger.snapshot()),/1 次用量未知/);
 assert.equal(cloudUsageSummary(ledger.snapshot(),'Microphone').seconds,'0');
 responses.tasks[0].inconsistent=true;await ledger.flush();assert.equal(cloudUsageSummary(ledger.snapshot()).known,1);assert.equal(cloudUsageSummary(ledger.snapshot()).unknown,2);
 let resolve;hold=new Promise(r=>resolve=r);const late=ledger.flush();ledger.reset();resolve();await late;hold=null;assert.equal(ledger.snapshot().tasks.length,0);
 assert.notEqual(ledger.begin(),scope);responses.tasks=[row(4,2.125,'running')];await ledger.flush();
 const base={version:1,history:[],transcripts:[],spoken_dialogue:[],current:{question:'',answer:''},calls:0,usage:'未知'};
 assert.throws(()=>parseSessionSnapshot(base));assert.deepEqual(parseSessionSnapshot(base,true),base);
 const saved={...base,cloud_asr_usage:ledger.snapshot()};parseSessionSnapshot(saved);const recovered=recoverSessionSnapshot(saved,'synthetic');
 assert.equal(recovered.cloud_asr_usage.tasks[0].state,'interrupted');assert.equal(cloudUsageSummary(recovered.cloud_asr_usage).seconds,'2.125');
 ledger.restore(recovered.cloud_asr_usage);const before=calls.length;await ledger.flush();assert.equal(calls.length,before,'recovery made a native/provider call');
 ledger.begin();responses.tasks=[row(5,3)];await ledger.flush();assert.equal(cloudUsageSummary(ledger.snapshot()).seconds,'5.125');
 for(const bad of [{version:2,tasks:[]},{version:1,tasks:[row(1,-1)]},{version:1,tasks:[row(1,Infinity)]},{version:1,tasks:[row(1,null),row(1,null)]}])assert.throws(()=>parseCloudUsage(bad));
 const huge={version:1,tasks:[row(6,Number.MAX_SAFE_INTEGER),row(7,1)]};assert.equal(cloudUsageSummary(huge).seconds,'9007199254740992');
 ledger.restore();assert.equal(ledger.snapshot().history_missing,true);assert.match(cloudUsageLabel(ledger.snapshot()),/旧记录用量未知/);ledger.reset();assert.equal(cloudUsageLabel(ledger.snapshot()),'');
 ledger.restore({version:1,tasks:Array.from({length:17953},(_,i)=>row(i+1,null))});assert.throws(()=>ledger.begin(),/已满/);
 console.log('PASS: ASR cumulative replacement, dual sources, zero/unknown/error, clear/late isolation, cloud-only persistence, recovery without calls, exact integer totals');
})().catch(e=>{console.error(e);process.exitCode=1;});
