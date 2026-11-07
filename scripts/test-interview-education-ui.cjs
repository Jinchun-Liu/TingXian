// Actual React panel with synthetic in-memory native bridge. No media or API.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=req('react'),{act,create}=req('react-test-renderer'),ts=req('typescript');
global.crypto=require('node:crypto').webcrypto;global.window={addEventListener(){},removeEventListener(){}};
const records=new Map(),calls=[],guards=new Set();let failSave=false,loseSaveReply=false,sessionGate=null;
const oldSnapshot={version:1,history:[{id:'old-q',role:'remote',question:'合成旧题',suggestion:'AI 草稿',confirmed_answer:null,status:'done',sources:[],time:'00:01',timing:''}],transcripts:[],spoken_dialogue:[],current:{question:'',answer:''},calls:0,usage:'未知'};
async function invoke(name,args){calls.push(name);
  if(name==='education_cancel')return;
  if(name==='education_list')return [...records.values()].map(s=>({id:s.id,question:s.draft.question,revision:s.store_revision,status:s.status}));
  if(name==='education_save'){if(failSave)throw Error('synthetic disk failure');const d=structuredClone(args.document);if(!Number.isInteger(d.preferences.context_limit)||d.preferences.context_limit<8192||d.preferences.context_limit>262144)throw Error('synthetic invalid model window');assert.equal(args.expected,records.get(d.id)?.store_revision??0);d.store_revision=args.expected+1;records.set(d.id,d);if(loseSaveReply){loseSaveReply=false;throw Error('synthetic lost save reply');}return d.store_revision;}
  if(name==='education_save_receipt'){const stored=records.get(args.document.id),candidate={...structuredClone(args.document),store_revision:stored?.store_revision??0};if(stored&&JSON.stringify(stored)===JSON.stringify(candidate))return stored.store_revision;assert.equal(args.expected,stored?.store_revision??0);return null;}
  if(name==='education_load')return structuredClone(records.get(args.id));
  if(name==='interview_providers_get')return {config:{answers:[],background:{enabled:false}},key_configured:{}};
  if(name==='tingxian_session_preview')return sessionGate?await sessionGate:structuredClone(oldSnapshot);
  throw Error('Unexpected real operation '+name);
}
const cache={};function load(file){if(cache[file])return cache[file];if(file.endsWith('.json'))return JSON.parse(fs.readFileSync(file,'utf8'));
  const m={exports:{}};const code=ts.transpileModule(fs.readFileSync(file,'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX,esModuleInterop:true}}).outputText;
  new Function('require','module','exports',code)(name=>{
    if(name==='@tauri-apps/api/core')return {invoke,isTauri:()=>true};
    if(name==='@/lib/interview-exit')return {interviewExit:{protect:fn=>{guards.add(fn);return()=>guards.delete(fn);}}};
    if(name==='@/components/ui/button')return {Button:({variant,size,...p})=>React.createElement('button',p)};
    if(name.startsWith('@/'))return load(path.resolve(__dirname,'../frontend/src',name.slice(2))+'.ts');
    if(name.startsWith('.'))return load(path.resolve(path.dirname(file),name)+(name.endsWith('.json')?'':'.ts'));
    return req(name);
  },m,m.exports);return cache[file]=m.exports;
}
const {EducationPanel}=load(path.resolve(__dirname,'../frontend/src/components/interview/EducationPanel.tsx'));
const text=n=>typeof n==='string'?n:(n?.children||[]).map(text).join('');
let tree,closed=0;const props={seed:null,visible:true,assistActive:false,scopeEpoch:'0',onClose:()=>closed++,onConfigure:()=>{}};
const field=name=>tree.root.findByProps({'aria-label':name});
const button=name=>tree.root.findAllByType('button').find(b=>text(b)===name);
const settle=async()=>{await new Promise(r=>setTimeout(r,20));};
async function change(name,value){await act(async()=>{field(name).props.onChange({target:{value}});await settle();});}
async function click(name){assert.ok(button(name),name);await act(async()=>{button(name).props.onClick();await settle();});}
(async()=>{
  await act(async()=>{tree=create(React.createElement(EducationPanel,props));await settle();});
  await change('复盘题目文字','StockMarketAgent 的提交后超时如何处理？');await change('我的练习回答','先查询幂等键对应的事务结果，再决定重试。');await change('选定资料文字','尚未加入发送范围的合成片段');await click('保存初答');
  const first=[...records.values()].find(s=>s.attempts.length);assert.ok(first);assert.equal(first.draft.source_draft,'尚未加入发送范围的合成片段');assert.equal(first.attempts.length,1);assert.equal(button('预览本次反馈范围').props.disabled,true);
  await click('再试一次');assert.equal(field('我的练习回答').props.value,'');await change('我的练习回答','第二次作答：核验写入和响应分别发生的状态。');await click('保存新的作答');assert.equal(records.get(first.id).attempts.length,2);
  await click('修订这次回答');await change('我的练习回答','第二次作答修订：加入租户与请求体哈希验证。');await click('保存修订');assert.equal(records.get(first.id).attempts[1].revisions.length,2);assert.equal(records.get(first.id).attempts[0].revisions[0].text,first.attempts[0].revisions[0].text);
  await click('导出当前复盘');assert.ok(field('导出预览'));await click('取消导出');
  await click('新练习');await change('打开练习记录',first.id);assert.equal(field('我的练习回答').props.value,'第二次作答修订：加入租户与请求体哈希验证。');assert.equal(field('选定资料文字').props.value,'尚未加入发送范围的合成片段');
  await change('复盘字号','24');assert.equal(tree.root.findByProps({className:'education-content'}).props.style.fontSize,24);
  failSave=true;await change('我的练习回答','磁盘失败时保留的文字');await click('保存草稿');assert.match(text(tree.toJSON()),/保存未完成/);assert.equal(field('我的练习回答').props.value,'磁盘失败时保留的文字');await assert.rejects([...guards][0](),/disk failure/);assert.equal(closed,0);failSave=false;await click('重试保存');
  loseSaveReply=true;await change('我的练习回答','已提交但回执丢失的草稿');await click('保存草稿');assert.match(text(tree.toJSON()),/保存未完成/);
  await change('我的练习回答','回执丢失后继续输入的草稿');await click('重试保存');assert.equal(records.get(first.id).draft.text,'回执丢失后继续输入的草稿');assert.doesNotMatch(text(tree.toJSON()),/保存未完成/);assert.equal(records.get(first.id).attempts.length,2,'save recovery must not duplicate attempts');
  const validWindow=records.get(first.id).preferences.context_limit;
  await change('模型上下文上限','');await click('保存草稿');assert.match(text(tree.toJSON()),/invalid model window/);assert.equal(records.get(first.id).preferences.context_limit,validWindow,'invalid preference must not replace the recoverable record');
  await change('模型上下文上限','16384');await click('重试保存');assert.equal(records.get(first.id).preferences.context_limit,16384);assert.doesNotMatch(text(tree.toJSON()),/保存未完成/);
  const oldGeneration=records.get(first.id).generation;await act(async()=>{tree.update(React.createElement(EducationPanel,{...props,scopeEpoch:'1'}));await settle();});await click('保存草稿');assert.ok(records.get(first.id).generation>oldGeneration);
  const seed={key:'old-session',sourceRef:'saved:synthetic',turnId:'q-old',history:[{id:'q-old',role:'remote',question:'原会话问题',suggestion:'AI 编造经历不可作为初答',confirmed_answer:null,sources:[]}]};
  await act(async()=>{tree.update(React.createElement(EducationPanel,{...props,seed}));await settle();});assert.equal(field('我的练习回答').props.value,'');assert.match(text(tree.toJSON()),/缺少本人作答/);assert.equal(button('保存初答').props.disabled,true);
  // Selecting an old attempt must bind actions to that question, not the draft
  // and source currently left over from another question in the same review.
  const multiSeed={key:'two-question-review',sourceRef:'saved:synthetic-two',turnId:'q-a',history:[
    {id:'q-a',role:'remote',question:'合成题 A：事务提交后超时如何核验？',sources:[]},
    {id:'self-a',role:'self',question:'先核验租户与幂等键对应的写入结果。',sources:[]},
    {id:'q-b',role:'remote',question:'合成题 B：如何区分 null、0 和缺失字段？',sources:[]},
    {id:'self-b',role:'self',question:'分别核验 Schema 与业务语义。',sources:[]},
  ]};
  await act(async()=>{tree.update(React.createElement(EducationPanel,{...props,seed:multiSeed}));await settle();});
  await change('选取本人回答','self-a');await change('帮助程度','worked_example');await click('保存初答');
  await change('复盘题目','q-b');await change('选取本人回答','self-b');await click('保存新的作答');
  const multi=[...records.values()].find(s=>s.source_session_ref===multiSeed.sourceRef),originalB=structuredClone(multi.attempts[1]);
  async function selectAttempt(index){await act(async()=>{tree.root.findAllByType('button').filter(b=>text(b)==='查看这次反馈')[index].props.onClick();await settle();});}
  await selectAttempt(0);await click('再试一次');
  assert.equal(field('复盘题目文字').props.value,multiSeed.history[0].question,'re-answer must target the selected historical question');
  assert.equal(field('复盘题目').props.value,'q-a','source selection must follow the chosen question');
  await change('我的练习回答','重新作答 A：超时不等于事务失败。');await click('保存新的作答');
  let saved=records.get(multi.id);assert.equal(saved.attempts.length,3);assert.equal(saved.attempts[2].question_id,'q-a');assert.equal(saved.attempts[2].revisions[0].source_ref,null,'new typed answer must not claim an old utterance');assert.equal(saved.attempts[2].assistance_level,'worked_example');assert.deepEqual(saved.attempts[1],originalB);
  await selectAttempt(1);await click('修订这次回答');await change('我的练习回答','修订 B：保留 null、0、缺失字段的区别。');await click('保存修订');
  saved=records.get(multi.id);assert.equal(saved.draft.question_ref,'q-b');assert.equal(saved.draft.source_ref,'self-b');assert.equal(saved.attempts[1].revisions[1].source_ref,'self-b','correction keeps the selected original source');assert.deepEqual(saved.attempts[1].revisions[0],originalB.revisions[0]);
  await click('新练习');await change('打开练习记录',multi.id);await selectAttempt(0);await click('再试一次');
  assert.equal(field('复盘题目文字').props.value,multiSeed.history[0].question);await click('保存草稿');assert.equal(records.get(multi.id).draft.question_ref,'q-a');
  assert.ok(!calls.includes('education_generate'));assert.ok(!calls.some(n=>/audio|camera|answer_start/.test(n)));
  await click('返回对话');assert.equal(closed,1);await act(async()=>{tree.unmount();await settle();});assert.equal(guards.size,0);
  const {SavedSessionPreview}=load(path.resolve(__dirname,'../frontend/src/components/interview/SavedSessionPreview.tsx'));let reviewed=null,release;
  await act(async()=>{tree=create(React.createElement(SavedSessionPreview,{id:1,onReview:s=>reviewed=s}));});await click('复盘这份会话');assert.deepEqual(reviewed,oldSnapshot);assert.ok(!calls.includes('interview_session_load'),'review must not restore over live assistance');
  reviewed=null;sessionGate=new Promise(r=>release=r);await act(async()=>{button('复盘这份会话').props.onClick();await settle();});await act(async()=>{tree.unmount();release(oldSnapshot);await settle();});assert.equal(reviewed,null,'late read after navigating away must not reopen education');
  console.log('PASS: actual education panel initial/re-answer/revision, historical question/source binding before and after restore, no-model state, export preview, saved recovery, source draft, font, disk failure, mode generation, old AI-only session and quit protection; synthetic native bridge only');
})().catch(e=>{console.error(e);process.exitCode=1;});
