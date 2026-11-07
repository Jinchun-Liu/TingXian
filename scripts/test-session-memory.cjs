const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const ts=require('../frontend/node_modules/typescript');
const source=fs.readFileSync(path.join(__dirname,'../frontend/src/lib/interview-memory.ts'),'utf8');
const out={exports:{}};new Function('module','exports',ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText)(out,out.exports);
const {SessionMemory,memoryBoundaryValid}=out.exports;
const event=(n,text='不能把 null 当作 0',more={})=>({sequence_id:n,utterance_id:n,source:'System',text,is_partial:false,capture_session:1,native_session_id:2,audio_start_time:0,audio_end_time:1,timestamp:'synthetic',...more});
function fixture(enabled=true){let count=0,fail=false,delay=null;const rows=new Map(),calls=[];const invoke=async(name,args)=>{
 calls.push({name,args});if(name==='interview_memory_open')return enabled?{id:'session',epoch:1,generation:'main'}:null;
 if(name==='interview_memory_append'){
  if(delay)await delay;if(fail)throw Error('synthetic disk full');const r=args.input,old=rows.get(r.id);
  if(old&&old.expected_revision===r.expected_revision+1&&old.text===r.text&&old.closed===r.closed)return {persisted:true,event:{revision:old.expected_revision}};
  assert.equal(r.expected_revision,old?.expected_revision??0);rows.set(r.id,{...r,expected_revision:r.expected_revision+1});return {persisted:true,event:{revision:r.expected_revision+1}};
 }
 if(name==='interview_memory_raw_get')return rows.get(args.id);
 if(name==='interview_memory_boundary')return {session_id:'session',epoch:1,last_record:rows.size,last_sha256:'a'.repeat(64),checkpoint:null};
 if(name==='interview_memory_restore')return {id:'fork',epoch:1,generation:'forked'};
 if(name==='interview_memory_tick')return false;
 throw Error(name);
 };return {m:new SessionMemory(invoke,()=>`uuid-${++count}`),rows,calls,setFail:v=>fail=v,setDelay:v=>delay=v};}
(async()=>{
 const disabled=fixture(false);await disabled.m.ensure();for(let n=0;n<140;n++)await disabled.m.transcript(event(n));await disabled.m.tick(true);assert.deepEqual(disabled.calls.map(c=>c.name),['interview_memory_open']);assert.equal(await disabled.m.boundary(),null);
 const f=fixture();await f.m.ensure();f.m.beginCapture();let visible=[];
 for(let n=0;n<140;n++){f.m.dispatch(async()=>{await f.m.transcript(event(n));visible=[...visible,n].slice(-120);},e=>{throw e;});if(n%50===49)await f.m.flush();}
 await f.m.flush();assert.equal(f.rows.size,140);assert.equal(visible.length,120);assert.equal(visible[0],20);assert(f.rows.values().next().value.text.includes('null'));
 const before=[...visible];f.setFail(true);let warnings=0;
 f.m.dispatch(async()=>{await f.m.transcript(event(141));visible=[...visible,141].slice(-120);},()=>warnings++);
 f.m.dispatch(async()=>{await f.m.end({source:'System',utterance_id:141,success:true});},()=>warnings++);
 await assert.rejects(f.m.flush(),/disk full/);assert.deepEqual(visible,before);assert.equal(warnings,1);assert.equal(f.m.pendingCount,2);
 f.setFail(false);await f.m.retry();assert.equal(visible.at(-1),141);assert.equal(f.rows.size,141);assert.equal([...f.rows.values()].at(-1).closed,true);
 const prefix=event(200,'企业测试',{utterance_id:200}),second=event(201,'null 不能为零',{utterance_id:200});
 await f.m.transcript(prefix);await f.m.transcript(second);
 const final=event(202,'契约测试：null、0 与缺失字段必须分开',{utterance_id:200,replaces_utterance:true});await f.m.transcript(final);await f.m.end({source:'System',utterance_id:200,success:true});
 assert.equal(f.rows.get(final.memory_event_id).text,final.text);assert.equal(f.rows.get(final.memory_event_id).expected_revision,4);await f.m.transcript(final);assert.equal(f.rows.get(final.memory_event_id).expected_revision,4,'duplicate final transcript does not reopen the completed utterance');
 await f.m.correct(final,'契约测试：null 不能等于数字零');assert.equal(f.rows.get(final.memory_event_id).text,'契约测试：null 不能等于数字零');
 const own=event(300,'我只做过模拟',{source:'Microphone'});await f.m.transcript(own);await f.m.end({source:'Microphone',utterance_id:300,success:true});await f.m.confirmed('本人确认：没有处理过真实事故');assert.equal(f.calls.filter(c=>c.name==='answer_start').length,0);
 const key=own.memory_event_id;f.m.beginCapture();const another=event(300,'另一轮',{source:'Microphone'});await f.m.transcript(another);assert.notEqual(another.memory_event_id,key);
 const boundary=await f.m.boundary();assert(memoryBoundaryValid(boundary));assert(!memoryBoundaryValid({...boundary,epoch:0}));assert(!memoryBoundaryValid({...boundary,checkpoint:{session_id:'other'}}));
 const legacy=fixture();const oldA=event(1,'早期采集',{capture_session:1,native_session_id:10}),oldB=event(1,'下一次采集',{capture_session:2,native_session_id:11});await legacy.m.ensure([oldA,oldB]);assert.equal(legacy.rows.size,2);assert.notEqual(oldA.memory_event_id,oldB.memory_event_id);assert.equal(legacy.calls[0].args.missingOlderHistory,true);
 const emptyLegacy=fixture();await emptyLegacy.m.restore(null,[]);assert.equal(emptyLegacy.calls[0].args.missingOlderHistory,true,'v1 with an empty retained ring still cannot claim complete older history');
 await f.m.restore(boundary,[]);assert.equal(f.m.handle.id,'fork');const ticks=f.calls.length;await f.m.tick(false);assert.equal(f.calls.length,ticks,'restoring never grants external-send consent');
 const delayed=fixture();await delayed.m.ensure();let resolve;delayed.setDelay(new Promise(r=>resolve=r));let shown=false;delayed.m.dispatch(async()=>{await delayed.m.transcript(event(1));shown=true;},()=>{});await Promise.resolve();assert.equal(shown,false);resolve();await delayed.m.flush();assert.equal(shown,true);
 const overflow=fixture();await overflow.m.ensure();overflow.setFail(true);let overflowWarnings=0;for(let i=0;i<130;i++)overflow.m.dispatch(()=>overflow.m.transcript(event(i)),()=>overflowWarnings++);await assert.rejects(overflow.m.flush());assert.equal(overflow.m.pendingCount,128);assert(overflowWarnings>0,'overflow is explicit and bounded');
 for(const page of ['interview','interview-workspace']){const text=fs.readFileSync(path.join(__dirname,`../frontend/src/app/${page}/page.tsx`),'utf8');assert(text.indexOf('await memory.current.transcript(event)')<text.indexOf('spoken.current = mergeSpokenTranscript'));assert(text.includes('await memory.current.end(event)'));assert(text.includes('memory.current.restore(saved.memory??null'));assert(text.includes('snapshot:await sessionSnapshot()'));}
 const display=fs.readFileSync(path.join(__dirname,'../frontend/src/app/interview-display/page.tsx'),'utf8');assert(!display.includes('interview_memory_'));
 console.log('PASS: default-off, 140 records before ring eviction, ordered ACK/end, disk-full retry, full ASR revision, own context, capture isolation, exact recovery boundary, bounded overflow, both routes, read-only display');
})().catch(e=>{console.error(e);process.exitCode=1;});
