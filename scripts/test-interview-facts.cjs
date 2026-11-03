// Real fact editor, with strict native contract and asynchronous failures.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const frontend=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=frontend('react'),{act,create}=frontend('react-test-renderer'),ts=frontend('typescript');
const saves=[],calls=[],busy=[];let failSave=false,failRefresh=false,pendingHistory;
let card={fact_id:'personal-fact-card',version:0,previous_version:null,text:'old synthetic fact',source_note:'legacy source',sources:[],reason:'baseline',confirmed_at:null,recorded_at:'2026-11-03T00:00:00.000Z'};
function load(relative){
 const module={exports:{}};
 const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src',relative),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX,esModuleInterop:true}}).outputText;
 const req=name=>{
  if(name==='@/components/ui/button')return {Button:props=>React.createElement('button',props)};
  if(name==='@tauri-apps/api/core')return {invoke:async(name,args)=>{calls.push({name,args});assert.equal(name,'profile_facts_history');if(pendingHistory)return new Promise(resolve=>{pendingHistory=resolve;});return args.before===null?{versions:[card],next_before:1}:{versions:[{...card,version:0,text:'old retained value',confirmed_at:null}],next_before:null};}};
  if(name.startsWith('@/lib/'))return load(name.slice(2)+'.ts');return frontend(name);
 };new Function('require','module','exports',code)(req,module,module.exports);return module.exports;
}
const {FactReview}=load('components/interview/FactReview.tsx');
const text=node=>typeof node==='string'?node:(node?.children||[]).map(text).join('');
const candidates=[{source_id:'E1',root:'/synthetic/root',path:'evidence.md',hash:'hash-A',location:'段落1',text:'actual indexed evidence',kind:'evidence'},{source_id:'P1',root:'/synthetic/root',path:'practice.md',hash:'hash-B',location:'段落2',text:'practice only',kind:'practice'}];
(async()=>{
 let tree,locked=false;
 const render=()=>tree.update(React.createElement(FactReview,props()));
 const props=()=>({card,candidates,locked,onBusyChange:value=>busy.push(value),onRefresh:async()=>{if(failRefresh)throw Error('refresh failed');render();return card;},onSave:async update=>{
  saves.push(update);if(failSave)throw Error('stale version, no overwrite');
  assert.equal(update.expected_version,card.version);assert.equal(update.confirmed,true);
  card={...card,version:card.version+1,previous_version:card.version,text:update.text,source_note:update.source_note,reason:update.reason,confirmed_at:'2026-11-03T00:00:01.000Z'};render();return card;
 }});
 await act(async()=>{tree=create(React.createElement(FactReview,props()));});
 const button=label=>tree.root.findAllByType('button').find(n=>text(n)===label);
 const click=async label=>{const b=button(label);assert.ok(b,label);assert.ok(!b.props.disabled,label+' disabled');await act(async()=>{await b.props.onClick();});};
 const draft=()=>tree.root.findByProps({placeholder:'只填写本人核对过的事实与来源，不粘贴 AI 建议。'});
 const edit=async(label,value)=>act(async()=>tree.root.findByProps({'aria-label':label}).props.onChange({target:{value}}));
 const confirm=()=>tree.root.findAllByType('label').find(n=>text(n).includes('确认保存此更正')).findByType('input');
 const setConfirmed=async()=>act(async()=>confirm().props.onChange({target:{checked:true}}));
 assert.equal(saves.length,0);assert.equal(button('确认保存事实新版本').props.disabled,true);assert.equal(confirm().props.checked,false);
 assert.equal(tree.root.findByProps({'aria-label':'事实来源 P1'}).props.disabled,true);
 await edit('事实更正来源','Synthetic source paragraph 1');await edit('事实更正原因','Correct inaccurate date');await setConfirmed();
 await act(async()=>draft().props.onChange({target:{value:'new checked fact'}}));assert.equal(confirm().props.checked,false,'editing must invalidate prior confirmation');
 await act(async()=>tree.root.findByProps({'aria-label':'事实来源 E1'}).props.onChange({target:{checked:true}}));await setConfirmed();
 await click('确认保存事实新版本');assert.equal(saves.length,1);assert.deepEqual(saves[0].sources,[{source_id:'E1',root:'/synthetic/root',path:'evidence.md',hash:'hash-A'}]);assert.equal(confirm().props.checked,false);assert.equal(draft().props.value,'new checked fact');assert.deepEqual(busy,[true,false]);
 await edit('事实更正原因','second correction');await act(async()=>draft().props.onChange({target:{value:'draft must survive failure'}}));await setConfirmed();failSave=true;
 await click('确认保存事实新版本');assert.equal(draft().props.value,'draft must survive failure');assert.ok(text(tree.toJSON()).includes('no overwrite'));assert.equal(confirm().props.checked,false);
 card={...card,version:3,text:'newer external version'};await act(async()=>render());assert.ok(text(tree.toJSON()).includes('不能直接覆盖'));assert.equal(draft().props.value,'draft must survive failure');assert.equal(button('确认保存事实新版本').props.disabled,true);
 failRefresh=true;await click('载入最新事实卡并替换编辑稿');assert.equal(draft().props.value,'draft must survive failure');
 failRefresh=false;await click('载入最新事实卡并替换编辑稿');assert.equal(draft().props.value,'newer external version');assert.equal(confirm().props.checked,false);
 await click('查看事实版本历史');await click('加载更早版本');assert.deepEqual(calls.map(c=>c.args.before),[null,1]);assert.ok(text(tree.toJSON()).includes('old retained value'));assert.equal(saves.length,2,'viewing or reloading must not save a fact');
 pendingHistory=true;await act(async()=>{void button('查看事实版本历史').props.onClick();});
 failSave=false;await edit('事实更正原因','new saved version');await setConfirmed();await click('确认保存事实新版本');
 await act(async()=>{const resolve=pendingHistory;pendingHistory=null;resolve({versions:[{...card,text:'late history must not appear'}],next_before:null});});
 assert.ok(!text(tree.toJSON()).includes('late history must not appear'));assert.equal(button('查看事实版本历史').props.disabled,false);
 locked=true;await act(async()=>render());assert.ok(draft().props.disabled);assert.ok(button('确认保存事实新版本').props.disabled);
 await act(async()=>tree.unmount());
 console.log('PASS: explicit fact confirmation, evidence-only references, edit resets consent, failed/stale save preserves draft, explicit reload, pagination, late history isolation, session lock');
})().catch(error=>{console.error(error);process.exitCode=1;});
