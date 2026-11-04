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
