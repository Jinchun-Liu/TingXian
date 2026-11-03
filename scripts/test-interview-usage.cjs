// Synthetic accounting and snapshot replay; no API, audio, or personal data.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const ts=require('../frontend/node_modules/typescript');
const modules={};
function load(name){
 if(modules[name])return modules[name];const m={exports:{}};
 const code=ts.transpileModule(fs.readFileSync(path.join(__dirname,'../frontend/src/lib',name+'.ts'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText;
 new Function('require','module','exports',code)(name=>load(name.split('/').at(-1)),m,m.exports);return modules[name]=m.exports;
}
const {InterviewUsage,parseUsageSnapshot,usageSummary,usageLabel}=load('interview-usage');
const {parseSessionSnapshot,recoverSessionSnapshot}=load('interview-session');
let ledger=new InterviewUsage();assert.match(usageLabel(ledger.snapshot()),/尚无调用记录/);
ledger.begin('q',0);assert.equal(ledger.accept('q',0),true);assert.equal(ledger.accept('q',0),false);
assert.equal(usageSummary(ledger.snapshot()).unobservedSubmissions,1);
ledger.event('q',0,{type:'request_sent',attempt:1});ledger.event('q',0,{type:'request_sent',attempt:1});
ledger.event('q',0,{type:'done',usage:{total_tokens:27}});ledger.event('q',0,{type:'done',usage:{total_tokens:27}});
assert.deepEqual(usageSummary(ledger.snapshot()),{knownTokens:'27',knownResponses:1,attempts:1,unknownAttempts:0,unobservedSubmissions:0,legacyCalls:0});
ledger.begin('q',1);ledger.accept('q',1);ledger.event('q',1,{type:'request_sent',attempt:1});ledger.event('q',1,{type:'request_sent',attempt:2});
ledger.event('q',1,{type:'done',usage:{total_tokens:13}});
assert.equal(usageSummary(ledger.snapshot()).knownTokens,'40');assert.equal(usageSummary(ledger.snapshot()).unknownAttempts,1);
assert.match(usageLabel(ledger.snapshot()),/已知累计 40 tokens.*1 次发送用量未知/);
ledger.begin('cancelled',2);ledger.accept('cancelled',2);ledger.event('cancelled',2,{type:'request_sent',attempt:1});ledger.finish('cancelled',2,'cancelled');
assert.equal(usageSummary(ledger.snapshot()).unknownAttempts,2);
// A legitimately late final usage improves totals without reviving the answer.
ledger.event('cancelled',2,{type:'done',usage:{total_tokens:4}});
assert.equal(ledger.snapshot().requests.at(-1).state,'cancelled');assert.equal(usageSummary(ledger.snapshot()).knownTokens,'44');
const broken=new InterviewUsage();broken.begin('broken',0);broken.accept('broken',0);
broken.event('broken',0,{type:'usage',attempt:1,usage:{total_tokens:16}});broken.event('broken',0,{type:'error'});
assert.equal(usageSummary(broken.snapshot()).knownTokens,'16');assert.equal(broken.snapshot().requests[0].state,'error');
assert.equal(usageSummary(broken.snapshot()).unknownAttempts,0);
const updates=new InterviewUsage();updates.begin('updates',0);updates.accept('updates',0);
for(const n of [0,5,8])updates.event('updates',0,{type:'usage',attempt:1,usage:{total_tokens:n}});
updates.event('updates',0,{type:'done',usage:{total_tokens:10}});updates.event('updates',0,{type:'done',usage:{total_tokens:10}});
assert.equal(usageSummary(updates.snapshot()).knownTokens,'10');
for(const value of [undefined,-1,NaN,Infinity,1.5,'7',Number.MAX_SAFE_INTEGER+1]){
 const invalid=new InterviewUsage();invalid.begin('bad',0);invalid.accept('bad',0);invalid.event('bad',0,{type:'done',usage:{total_tokens:value}});
 assert.equal(usageSummary(invalid.snapshot()).unknownAttempts,1);assert.match(usageLabel(invalid.snapshot()),/累计用量未知/);
}
const zero=new InterviewUsage();zero.begin('zero',0);zero.accept('zero',0);zero.event('zero',0,{type:'done',usage:{total_tokens:0}});assert.match(usageLabel(zero.snapshot()),/累计 0 tokens/);
const large=new InterviewUsage();for(let i=0;i<2;i++){large.begin('large',i);large.accept('large',i);large.event('large',i,{type:'done',usage:{total_tokens:Number.MAX_SAFE_INTEGER}});}
assert.equal(usageSummary(large.snapshot()).knownTokens,'18014398509481982');
assert.equal(ledger.event('not-this-session',0,{type:'done',usage:{total_tokens:999}}),false);
assert.throws(()=>parseUsageSnapshot(ledger.snapshot(),999));
const duplicate=ledger.snapshot();duplicate.requests.push(duplicate.requests[0]);assert.throws(()=>parseUsageSnapshot(duplicate,4));
const snapshot={version:1,history:[{id:'q',role:'remote',question:'synthetic',suggestion:'answer',confirmed_answer:null,status:'done',sources:[],time:'00:01',timing:''}],transcripts:[],spoken_dialogue:[],current:{question:'',answer:''},calls:3,usage:'4 tokens',usage_ledger:ledger.snapshot()};
assert.deepEqual(parseSessionSnapshot(snapshot).usage_ledger,ledger.snapshot());
const restored=recoverSessionSnapshot(snapshot,'restored-run');
assert.throws(()=>recoverSessionSnapshot(snapshot,'x'.repeat(1000)));
const restoredLedger=new InterviewUsage(restored.usage_ledger,restored.calls);
assert.equal(restoredLedger.event('q',0,{type:'done',usage:{total_tokens:999}}),false);
assert.equal(usageSummary(restoredLedger.snapshot()).knownTokens,'44');
const legacy={...snapshot};delete legacy.usage_ledger;
const legacyLedger=new InterviewUsage(parseSessionSnapshot(legacy).usage_ledger,legacy.calls);
assert.match(usageLabel(legacyLedger.snapshot()),/旧记录 3 次用量未知/);
assert.equal(usageSummary(legacyLedger.snapshot()).knownTokens,'0');
ledger=new InterviewUsage();assert.equal(ledger.event('q',0,{type:'done',usage:{total_tokens:999}}),false);
console.log('PASS: observed cumulative usage, retries/unknown/zero, duplicate and late events, revision isolation, precise totals, legacy/snapshot restoration and clear isolation');
