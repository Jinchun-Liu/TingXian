// Exercise the actual scene component with a synthetic native boundary.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const frontend=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=frontend('react'),{act,create}=frontend('react-test-renderer'),ts=frontend('typescript');
let stored={role:'品牌策划',organization:'示例组织',context:'零售行业',terminology:'品牌认知',answer_language:'en',transcription_language:'zh'};
let fail=false,pending=null,locked=false;const calls=[];let saved=null;
const invoke=async(name,args)=>{calls.push(name);if(name==='interview_preferences_get'){if(pending)return new Promise(resolve=>{pending=resolve;});return {...stored};}assert.equal(name,'interview_preferences_save');if(fail)throw Error('synthetic failure');stored={...args.preferences};return {...stored};};
const moduleObject={exports:{}};
const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/InterviewPreferences.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX,esModuleInterop:true}}).outputText;
new Function('require','module','exports',code)(name=>name==='@tauri-apps/api/core'?{invoke}:name==='@/components/ui/button'?{Button:props=>React.createElement('button',props)}:frontend(name),moduleObject,moduleObject.exports);
const {InterviewPreferences}=moduleObject.exports;
const text=node=>typeof node==='string'?node:(node?.children||[]).map(text).join('');
(async()=>{
 let tree;const render=()=>{const e=React.createElement(InterviewPreferences,{locked,onSaved:value=>{saved=value;}});tree?tree.update(e):tree=create(e);};
 await act(async()=>render());
 const input=label=>tree.root.findByProps({'aria-label':label});
 const button=label=>tree.root.findAllByType('button').find(n=>text(n)===label);
 const click=async label=>{const b=button(label);assert.ok(b && !b.props.disabled);await act(async()=>b.props.onClick());};
 assert.equal(input('目标岗位').props.value,'品牌策划');assert.equal(input('回答语言').props.value,'en');
 await act(async()=>input('目标岗位').props.onChange({target:{value:'策展助理'}}));
 fail=true;await click('保存面试场景');assert.equal(saved,null);assert.equal(input('目标岗位').props.value,'策展助理');assert.ok(text(tree.toJSON()).includes('synthetic failure'));
 fail=false;await click('保存面试场景');assert.equal(saved.role,'策展助理');assert.equal(stored.context,'零售行业');
 locked=true;await act(async()=>render());assert.ok(input('转写语言').props.disabled);assert.ok(button('保存面试场景').props.disabled);
 locked=false;await act(async()=>render());pending=true;await click('重新读取场景');assert.ok(button('保存面试场景').props.disabled);await click('恢复空白场景');
 await act(async()=>{const resolve=pending;pending=null;resolve({...stored,role:'stale'});});assert.equal(input('目标岗位').props.value,'');assert.equal(input('转写语言').props.value,'auto');
 await click('保存面试场景');assert.equal(stored.role,'');assert.equal(stored.terminology,'');
 assert.ok(calls.every(name=>['interview_preferences_get','interview_preferences_save'].includes(name)));
 await act(async()=>tree.unmount());console.log('PASS: scene load/edit/save/reset, errors, lock and stale reads; no capture or model calls');
})().catch(e=>{console.error(e);process.exitCode=1;});
