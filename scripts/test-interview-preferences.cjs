// Exercise the actual scene component with a synthetic native boundary.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const frontend=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=frontend('react'),{act,create}=frontend('react-test-renderer'),ts=frontend('typescript');
let stored={scenario:'interview',role:'品牌策划',organization:'示例组织',context:'零售行业',terminology:'品牌认知',answer_language:'en',transcription_language:'zh'};
let fail=false,pending=null,locked=false,blocked=[],sourceRevision='1';const calls=[];let saved=null;
const invoke=async(name,args)=>{
 calls.push(name);
 if(name==='interview_preferences_state'){if(pending)return new Promise(resolve=>{pending=resolve;});return {preferences:{...stored},blocked_fields:[...blocked]};}
 if(fail)throw Error('synthetic failure');
 if(name==='interview_preferences_manual'){
  assert.equal(args.fields.length,1);const {field,expected}=args.fields[0];assert.equal(stored[field],expected);
  blocked=blocked.filter(x=>x!==field);return {preferences:{...stored},blocked_fields:[...blocked]};
 }
 assert.equal(name,'interview_preferences_save');assert.deepEqual(args.expected,stored);blocked=blocked.filter(field=>args.preferences[field]===stored[field]&&args.preferences[field]!=='');stored={...args.preferences};return {...stored};
};
const sceneModule={exports:{}};
new Function('module','exports',ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/lib/conversation-scenarios.ts'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS}}).outputText)(sceneModule,sceneModule.exports);
const moduleObject={exports:{}};
const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/InterviewPreferences.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX,esModuleInterop:true}}).outputText;
new Function('require','module','exports',code)(name=>name==='@/lib/conversation-scenarios'?sceneModule.exports:name==='@tauri-apps/api/core'?{invoke}:name==='@/components/ui/choice-menu'?{ChoiceMenu:'choice-menu',ChoiceOption:'choice-option'}:name==='@/components/ui/button'?{Button:props=>React.createElement('button',props)}:frontend(name),moduleObject,moduleObject.exports);
const {InterviewPreferences}=moduleObject.exports;
const text=node=>typeof node==='string'?node:(node?.children||[]).map(text).join('');
(async()=>{
 let tree;const render=()=>{const e=React.createElement(InterviewPreferences,{locked,sourceRevision,onSaved:value=>{saved=value;}});tree?tree.update(e):tree=create(e);};
 await act(async()=>render());
 const input=label=>tree.root.findByProps({'aria-label':label});
 const button=label=>tree.root.findAllByType('button').find(n=>text(n)===label);
 const click=async label=>{const b=button(label);assert.ok(b && !b.props.disabled);await act(async()=>b.props.onClick());};
 assert.equal(input('角色 / 目标岗位').props.value,'品牌策划');assert.equal(input('回答语言').props.value,'en');
 await act(async()=>input('角色 / 目标岗位').props.onChange({target:{value:'待保存的岗位草稿'}}));blocked=['context'];sourceRevision='2';await act(async()=>render());assert.equal(input('角色 / 目标岗位').props.value,'待保存的岗位草稿');assert.ok(text(tree.toJSON()).includes('暂停用于 AI'));assert.ok(!button('保存对话场景').props.disabled);
 await act(async()=>input('角色 / 目标岗位').props.onChange({target:{value:'策展助理'}}));
 fail=true;await click('保存对话场景');assert.equal(saved,null);assert.equal(input('角色 / 目标岗位').props.value,'策展助理');assert.ok(text(tree.toJSON()).includes('synthetic failure'));
 fail=false;await click('保存对话场景');assert.equal(saved.role,'策展助理');assert.equal(stored.context,'零售行业');
 await act(async()=>input('使用场景').props.onValueChange('learning'));await click('保存对话场景');assert.equal(stored.scenario,'learning');
 locked=true;await act(async()=>render());assert.ok(input('转写语言').props.disabled);assert.ok(button('保存对话场景').props.disabled);
 locked=false;await act(async()=>render());pending=true;await click('重新读取场景');assert.ok(button('保存对话场景').props.disabled);await click('恢复空白场景');
 await act(async()=>{const resolve=pending;pending=null;resolve({preferences:{...stored,role:'stale'},blocked_fields:[]});});assert.equal(input('角色 / 目标岗位').props.value,'');assert.equal(input('转写语言').props.value,'auto');
 await click('保存对话场景');assert.equal(stored.role,'');assert.equal(stored.terminology,'');assert.equal(stored.scenario,'general');
 stored={...stored,context:'已撤回的背景',role:'本人岗位'};blocked=['context'];await click('重新读取场景');
 assert.ok(text(tree.toJSON()).includes('暂停用于 AI'));assert.equal(input('对话背景与关注点').props.value,'已撤回的背景');
 await act(async()=>input('回答语言').props.onValueChange('en'));await click('保存对话场景');assert.deepEqual(blocked,['context']);
 await act(async()=>input('角色 / 目标岗位').props.onChange({target:{value:'未保存的角色草稿'}}));
 fail=true;await click('转为手动设置');assert.deepEqual(blocked,['context']);fail=false;
 await click('转为手动设置');assert.deepEqual(blocked,[]);assert.equal(input('角色 / 目标岗位').props.value,'未保存的角色草稿');assert.equal(stored.role,'本人岗位');
 stored.role='另一处新应用的岗位';sourceRevision='3';await act(async()=>render());assert.equal(input('角色 / 目标岗位').props.value,'未保存的角色草稿');assert.ok(button('保存对话场景').props.disabled);await click('重新读取场景');assert.equal(input('角色 / 目标岗位').props.value,'另一处新应用的岗位');
 assert.ok(calls.every(name=>['interview_preferences_state','interview_preferences_save','interview_preferences_manual'].includes(name)));
 await act(async()=>tree.unmount());console.log('PASS: scene load/edit/save/reset, errors, lock, stale reads and explicit source review without losing drafts; no capture or model calls');
})().catch(e=>{console.error(e);process.exitCode=1;});
