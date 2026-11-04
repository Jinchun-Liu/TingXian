const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const ts=req('typescript'),React=req('react'),{create,act}=req('react-test-renderer');
const cache={};
function load(file){
 if(cache[file])return cache[file];
 const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src',file),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX}}).outputText;
 const m={exports:{}};new Function('require','module','exports',code)(name=>{
  if(name==='@/components/ui/choice-menu')return {ChoiceMenu:'choice-menu',ChoiceOption:'choice-option'};
    if(name==='@/components/ui/button')return {Button:p=>React.createElement('button',p)};
  if(name==='./WorkspaceFeed')return load('components/interview/WorkspaceFeed.tsx');
  if(name==='@/lib/interview-presentation')return load('lib/interview-presentation.ts');
  return req(name);
 },m,m.exports);return cache[file]=m.exports;
}
const {answerPresentation,AnswerRequests,visibleTurns,presentationDefaults}=load('lib/interview-presentation.ts');
const raw='建议回答：分别验证 null、0 和缺失字段。\n<<<QUESTION>>>\nExampleService 的契约测试如何区分 null、零与缺失字段？';
for(let i=0;i<=raw.length;i++){
 const result=answerPresentation(raw.slice(0,i),false,'原始误识别');
 assert.equal(result.question,'');assert.ok(!/[<>]|QUESTION|建议回答/u.test(result.answer));
 assert.ok(!result.answer.includes('ExampleService'));
}
assert.deepEqual(answerPresentation(raw,true,'原文'),{answer:'分别验证 null、0 和缺失字段。',question:'ExampleService 的契约测试如何区分 null、零与缺失字段？',corrected:true});
assert.equal(answerPresentation('建议回答：旧答案。\n听写纠正：零→0',true,'原文').answer,'旧答案。');
assert.equal(answerPresentation('建议回答：已有答案。\n<<<QUES',false,'原文').answer,'已有答案。');
assert.equal(answerPresentation('不带格式的旧答案',true,'原文').question,'原文');
const jobs=new AnswerRequests();const a=jobs.begin('a',3),b=jobs.begin('b',3),c=jobs.begin('c',3);
assert.equal(jobs.begin('d',3),null);assert.equal(jobs.begin('a',3),null);
assert.equal(jobs.append('b',b,'答案 B'),'答案 B');assert.equal(jobs.append('a',a,'答案 A'),'答案 A');
assert.equal(jobs.finish('b',b),true);assert.equal(jobs.append('b',b,'迟到'),null);assert.equal(jobs.size,2);
const d=jobs.begin('d',3);assert.ok(d>c);assert.equal(jobs.append('a',a,'继续'),'答案 A继续');
assert.equal(jobs.clear().length,3);assert.equal(jobs.append('d',d,'取消后迟到'),null);
const vision=jobs.begin('image',1,'vision');assert.ok(vision);assert.equal(jobs.begin('second-image',1,'vision'),null);
const textJob=jobs.begin('text',1);assert.ok(textJob,'image must not consume the sole text provider');
assert.equal(jobs.begin('second-text',1),null);assert.ok(jobs.begin('another-text',2));
assert.equal(jobs.begin('fourth',3),null);assert.equal(jobs.count('text'),2);assert.equal(jobs.count('vision'),1);jobs.clear();
let history=[{id:'a',role:'remote',question:'问题一',suggestion:raw,status:'done',time:'1'},{id:'self',role:'self',question:'我实际说了幂等键',suggestion:'',status:'done',time:'2'}];
assert.deepEqual(visibleTurns(history,false).map(t=>t.id),['a']);assert.equal(history.length,2,'display filter deleted source history');
const {AnswerConversation}=load('components/interview/AnswerConversation.tsx');
const text=n=>typeof n==='string'?n:(n?.children||[]).map(text).join('');
(async()=>{
 let tree,selected=null;let preferences={...presentationDefaults};
 const render=()=>React.createElement(AnswerConversation,{history,preferences,selectedId:selected,onSelect:id=>{selected=id;},onEdit:()=>{}});
 await act(async()=>{tree=create(render());});
 assert.ok(!text(tree.toJSON()).includes('我实际说了幂等键'));
 assert.ok(text(tree.toJSON()).indexOf('分别验证')<text(tree.toJSON()).lastIndexOf('ExampleService'));
 const article=tree.root.findByType('article');assert.equal(article.props.style.fontSize,20);
 await act(async()=>article.props.onPointerDown());assert.equal(selected,'a');
 history=[...history,{id:'b',role:'remote',question:'后来问题',suggestion:'建议回答：后来答案\n<<<QUESTION>>>\n后来问题',status:'done',time:'3'}];
 await act(async()=>tree.update(render()));assert.equal(tree.root.findByType('article').props['data-turn-id'],'a','new arrival replaced pinned question');
 selected=null;preferences={...preferences,fontSize:'large'};await act(async()=>tree.update(render()));
 assert.equal(tree.root.findByType('article').props['data-turn-id'],'b');assert.equal(tree.root.findByType('article').props.style.fontSize,24);
 preferences={...preferences,showSelf:true,focus:false};await act(async()=>tree.update(render()));assert.ok(text(tree.toJSON()).includes('我实际说了幂等键'));
 await act(async()=>tree.unmount());
 console.log('PASS: protocol chunk boundaries, answer first/corrected question last, legacy fallback, three isolated streams, stale/cancel suppression, self hidden without deletion, stable pinned question and font sizes');
})().catch(e=>{console.error(e);process.exitCode=1;});
