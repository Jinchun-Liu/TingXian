// Actual alias editor, synthetic IPC only; no browser/network/application DB.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const frontend=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=frontend('react'),{act,create}=frontend('react-test-renderer'),ts=frontend('typescript');
let stored='企业,契约\n本人,manual',blocked=['企业,契约'],fail=false,holdSave=null,holdRead=null,locked=false,sourceRevision='1';
const calls=[];
const invoke=async(name,args)=>{
 calls.push({name,args});
 if(name==='interview_aliases_state') {if(holdRead)return new Promise(resolve=>{holdRead=resolve;});return {aliases:stored,blocked_lines:[...blocked]};}
 if(fail)throw Error('synthetic failure');
 assert.equal(args.expected,stored);
 if(name==='interview_aliases_manual'){blocked=blocked.filter(line=>!args.lines.includes(line));return;}
 assert.equal(name,'interview_aliases_save');if(holdSave)await new Promise(resolve=>{holdSave=resolve;});
 stored=args.aliases;blocked=blocked.filter(line=>stored.split('\n').map(s=>s.trim()).includes(line));
};
const moduleObject={exports:{}};
const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/AliasSettings.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX,esModuleInterop:true}}).outputText;
new Function('require','module','exports',code)(name=>name==='@tauri-apps/api/core'?{invoke}:name==='@/components/ui/button'?{Button:props=>React.createElement('button',props)}:frontend(name),moduleObject,moduleObject.exports);
const {AliasSettings}=moduleObject.exports;
const text=node=>typeof node==='string'?node:(node?.children||[]).map(text).join('');
(async()=>{
 let tree;const render=()=>{const e=React.createElement(AliasSettings,{locked,sourceRevision});tree?tree.update(e):tree=create(e);};
 await act(async()=>render());
 const input=()=>tree.root.findByProps({'aria-label':'术语别名'});
 const button=label=>tree.root.findAllByType('button').find(n=>text(n)===label);
 const click=async label=>{const b=button(label);assert.ok(b&&!b.props.disabled);await act(async()=>b.props.onClick());};
 assert.equal(input().props.value,stored);assert.ok(text(tree.toJSON()).includes('已暂停检索'));
 await act(async()=>input().props.onChange({target:{value:stored+'\n新词,new'}}));assert.ok(button('转为手动别名').props.disabled);
 fail=true;await click('保存别名');assert.ok(text(tree.toJSON()).includes('synthetic failure'));assert.ok(input().props.value.endsWith('新词,new'));
 fail=false;await click('保存别名');assert.deepEqual(blocked,['企业,契约']);
 await click('转为手动别名');assert.deepEqual(blocked,[]);
 await act(async()=>input().props.onChange({target:{value:'未保存草稿'}}));blocked=['企业,契约'];sourceRevision='2';await act(async()=>render());
 assert.equal(input().props.value,'未保存草稿');assert.ok(!button('保存别名').props.disabled);
 stored+='\n自动加入,added';sourceRevision='3';await act(async()=>render());assert.equal(input().props.value,'未保存草稿');assert.ok(button('保存别名').props.disabled);
 await click('重新读取别名');assert.equal(input().props.value,stored);
 // Library revision arriving during save must not orphan the pending action.
 holdSave=true;await click('保存别名');sourceRevision='4';await act(async()=>render());
 await act(async()=>{const resolve=holdSave;holdSave=null;resolve();});assert.ok(!button('保存别名').props.disabled);
 locked=true;await act(async()=>render());assert.ok(input().props.disabled);assert.ok(button('重新读取别名').props.disabled);
 locked=false;await act(async()=>render());
 holdRead=true;await click('重新读取别名');const late=holdRead;holdRead=null;
 // A superseded read cannot overwrite a later explicit reload.
 await click('重新读取别名');await act(async()=>late({aliases:'STALE',blocked_lines:[]}));assert.equal(input().props.value,stored);
 assert.ok(calls.every(c=>c.name.startsWith('interview_aliases_')));
 await act(async()=>tree.unmount());console.log('PASS: alias source review, changed drafts, CAS save, lifecycle locks, failures and stale reads; no model calls');
})().catch(e=>{console.error(e);process.exitCode=1;});
