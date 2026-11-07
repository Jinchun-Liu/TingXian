const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const ts=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'))('typescript');
function load(name){const m={exports:{}};const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/lib/'+name+'.ts'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText;new Function('exports','module',code)(m.exports,m);return m.exports;}
const {MeetingAddress}=load('meeting-mode'),{RemoteUtteranceQueue}=load('interview-scheduler');
for(const mode of ['pair','panel','group','groups','mixed']){
 const names='小林、Alex、三号候选人',gate=new MeetingAddress(),queue=new RemoteUtteranceQueue(),alerts=[];
 const route=(text,time)=>{const r=gate.decide({mode,names},text,time);if(r.alert)alerts.push(r.alert);return r.answer;};
 let seq=0;
 function turn(text,time,{refine,source='System',success=true,allow=true}={}){const id=++seq;
  const send=(words,replaces=false)=>queue.receive({utterance_id:id,source,text:words,is_partial:false,sequence_id:++seq,timestamp:'12:00',replaces_utterance:replaces});
  send(text);if(refine)send(refine,true);
  const end={source,utterance_id:id,success};const result=queue.finish(end,allow,t=>route(t,time));
  assert.equal(queue.finish(end,true,t=>route(t,time)).status,'ignored');return result.status;
 }
 const gated=!['pair','panel'].includes(mode);
 assert.equal(turn('请王老师说说计划。',0),gated?'recorded':'queued');
 assert.equal(turn('小林，请解释你的方案？',1,{source:'Microphone'}),'ignored');
 assert.equal(turn('小林，请回答。',2,{refine:'这是小林的方案，接下来请王老师介绍。'}),gated?'recorded':'queued');
 assert.equal(turn('“小林，请说说进度”，是一个点名示例。',3),gated?'recorded':'queued');
 assert.equal(turn('小林，你先不用回答，请小王来。',3),gated?'recorded':'queued');
 assert.equal(turn('Alexandra, what changed?',4),gated?'recorded':'queued');
 assert.equal(turn('小林，请解释你的方案？',5),'queued');
 assert.equal(turn('Alex。',6),gated?'recorded':'queued');
 assert.equal(turn('Could you explain the delay?',7),'queued');
 assert.equal(turn('请评价一下。',8),gated?'recorded':'queued');
 assert.equal(turn('三号候选人。',9),gated?'recorded':'queued');
 assert.equal(turn('你怎么看？',14000),gated?'recorded':'queued');
 assert.equal(turn('Alex, what changed?',15000,{allow:false}),'recorded');
 assert.equal(turn('小林，请回答',16000,{success:false}),'failed');
 assert.deepEqual(alerts,gated?['小林','alex','三号候选人','alex']:[]);
}
console.log('PASS: five modes, own voice, unaddressed speech, refined text, quotes, English boundaries, separate call/question, expiry, permission and duplicate completion');

// Synthetic routing regressions, not the PRD's 100 human-labelled examples.
const {meetingNotice}=load('meeting-mode');
const delayed=[
 '小林，请稍后总结。','小林，你等大家说完再回答。','稍后请小林总结。',
 '小林，请你待会儿分享。','小林，等其他人讲完再回答。',
 'Alex, please summarize the discussion later.','Alex, after everyone has spoken, please summarize.',
];
const invited=['大家各说一句。','接下来，请各位依次发言。','我们轮流介绍一下。','Everyone, please take turns.'];
const nowQuestions=[
 '小林，请解释为什么任务要稍后执行。','小林，你如何设计延迟队列，让任务稍后运行？',
 '小林，请介绍 Retry-After 和 HTTP 429 的关系。',
 'Alex, please answer whether we should retry later?',
 '小林，请最后解释为什么发生超时。',
 '小林，现在请你总结。','Alex, how does retry backoff work?',
];
const mentions=['这是小林稍后要介绍的方案。','“小林，请稍后总结”，只是一个示例。','“大家各说一句”，这句话不用执行。','Alexandra, please summarize later.'];
for(const mode of ['pair','panel','group','groups','mixed']){
 const setup={mode,names:'小林、Alex'},gated=['group','groups','mixed'].includes(mode);
 for(const text of delayed){const result=new MeetingAddress().decide(setup,text,0);assert.equal(result.answer,!gated,text);assert.equal(result.reminder,gated?'later':undefined,text);}
 for(const text of invited){const result=new MeetingAddress().decide(setup,text,0);assert.equal(result.answer,!gated,text);assert.equal(result.reminder,gated?'collective':undefined,text);}
 for(const text of nowQuestions)assert.equal(new MeetingAddress().decide(setup,text,0).answer,true,text);
 for(const text of mentions)assert.equal(new MeetingAddress().decide(setup,text,0).answer,!gated,text);
 if(!gated)continue;
 let gate=new MeetingAddress();gate.decide(setup,'小林。',10);
 assert.equal(gate.decide(setup,'请稍后总结。',20).reminder,'later');
 assert.equal(gate.decide(setup,'你怎么看？',30).answer,false,'deferred instruction must not open a 12-second answer window');
 assert.equal(gate.decide(setup,'小林，现在请你总结。',40).answer,true);
 gate.decide(setup,'小林。',50);assert.equal(gate.decide(setup,'你先不用回答。',60).reminder,'cancelled');
 assert.equal(gate.decide(setup,'请说说。',70).answer,false,'decline consumed the pending call');
 gate.decide(setup,'小林。',100);gate.reset();assert.equal(gate.decide(setup,'请稍后总结。',110).reminder,undefined);
 gate.decide(setup,'小林。',200);assert.equal(gate.decide(setup,'请稍后总结。',12200).reminder,undefined,'expired call cannot schedule a reminder');
 gate=new MeetingAddress();const queue=new RemoteUtteranceQueue();
 queue.receive({sequence_id:1,text:'小林，请回答。',source:'System',utterance_id:1,is_partial:false,timestamp:'12:00'});
 queue.receive({sequence_id:2,text:'小林，请稍后回答。',source:'System',utterance_id:1,is_partial:false,replaces_utterance:true,timestamp:'12:00'});
 assert.equal(queue.finish({source:'System',utterance_id:1,success:true},true,t=>gate.decide(setup,t,0).answer).status,'recorded');assert.equal(queue.size,0);
}
assert.equal(new MeetingAddress().decide({mode:'groups',names:''},invited[0],0).reminder,undefined);
assert.deepEqual(meetingNotice({answer:false,alert:'小林',reminder:'later'}),{text:'稍后发言 · 小林',persistent:true});
assert.deepEqual(meetingNotice({answer:false,alert:'',reminder:'cancelled'}),{text:'',persistent:false});
console.log('PASS: delayed/collective invitations only remind; immediate technical questions, quotes, final ASR revisions, expiry and cancel preserve routing');

// Real React hook, fake storage/clock only. No native bridge, speech or model.
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=req('react'),{act,create}=req('react-test-renderer');
const compiled=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/MeetingMode.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX}}).outputText;
const hookModule={exports:{}};new Function('require','module','exports',compiled)(name=>name==='@/lib/meeting-mode'?load('meeting-mode'):name==='@/components/ui/choice-menu'?{}:req(name),hookModule,hookModule.exports);
(async()=>{
 const original={setTimeout:global.setTimeout,clearTimeout:global.clearTimeout,localStorage:global.localStorage};let timers=new Map(),timerId=0,store=new Map();
 global.setTimeout=(fn,ms)=>{const id=++timerId;timers.set(id,{fn,ms});return id;};global.clearTimeout=id=>timers.delete(id);
 global.localStorage={getItem:k=>store.get(k)||null,setItem:(k,v)=>store.set(k,v)};
 let api,tree;function Probe(){api=hookModule.exports.useMeetingMode(()=>{});return null;}
 const speak=async(text)=>{let result;await act(async()=>{result=api.decide(text);});return result;};
 try{
  await act(async()=>{tree=create(React.createElement(Probe));});
  await act(async()=>{api.change({mode:'groups',names:'小林、Alex'});});
  assert.equal(await speak('小林，请稍后总结。'),false);assert.equal(api.notice,'稍后发言 · 小林');assert.equal(timers.size,0,'deferred task must not silently expire after 12 seconds');
  assert.equal(await speak('大家各说一句。'),false);assert.equal(api.notice,'稍后发言 · 小林','group invitation must not erase deferred task');
  assert.equal(await speak('小林，现在请你总结。'),true);assert.equal(api.notice,'正在叫你 · 小林');assert.equal(timers.size,1);
  await act(async()=>{api.interrupt();});assert.equal(api.notice,'');assert.equal(timers.size,0,'own speech cancels stale reminders');
  await speak('小林，请稍后总结。');await speak('小林，请不用回答了。');assert.equal(api.notice,'');
  await speak('大家各说一句。');assert.equal(api.notice,'等待轮次 · 大家依次发言');assert.equal(timers.size,1);
  await act(async()=>{for(const [id,t] of timers){timers.delete(id);t.fn();}});assert.equal(api.notice,'');
  await speak('小林，请稍后总结。');await act(async()=>{api.change({mode:'pair',names:'小林'});});assert.equal(api.notice,'');assert.equal(timers.size,0);
  await act(async()=>{api.change({mode:'mixed',names:'小林'});});await speak('小林。');await act(async()=>{api.interrupt();});assert.equal(await speak('请介绍方案。'),false);
  await speak('小林，请稍后总结。');await act(async()=>{api.reset();});assert.equal(api.notice,'');
  console.log('PASS: shared hook keeps deferred reminder, supersedes on direct call, clears on own speech/mode/session reset, and expires only transient notices');
 } finally {if(tree)await act(async()=>tree.unmount());Object.assign(global,original);}
})().catch(e=>{console.error(e);process.exitCode=1;});
