const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=req('react'),{act,create}=req('react-test-renderer'),ts=req('typescript');
const view={version:'8',profile_hash:'synthetic-hash',current:{role:'手填岗位',organization:'手填组织',context:'手填背景',terminology:'手填术语'},suggestions:{role:'建议岗位',organization:'建议组织',context:'仅为练习经历',terminology:'JSON Schema',call_names:'小林'}};
const calls=[];let fail=false,gate=null,resolveGate,names='原称呼',namesCalls=0;
const invoke=async(cmd,args)=>{
 calls.push({cmd,args});if(cmd==='background_settings_preview')return structuredClone(view);
 assert.equal(cmd,'background_settings_apply');if(gate)await gate;if(fail)throw Error('所选设置已被手动修改');
 for(const c of args.changes)view.current[c.field]=c.value;
};
const load=relative=>{const m={exports:{}};const src=fs.readFileSync(path.resolve(__dirname,'../frontend/src',relative),'utf8');
 new Function('require','module','exports',ts.transpileModule(src,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX}}).outputText)(n=>{
  if(n==='@tauri-apps/api/core')return {invoke};if(n==='@/components/ui/button')return {Button:'button'};if(n==='@/components/ui/choice-menu')return {ChoiceMenu:'choice',ChoiceOption:'option'};
  if(n==='@/lib/meeting-mode')return load('lib/meeting-mode.ts');return req(n);
 },m,m.exports);return m.exports;
};
const text=n=>typeof n==='string'?n:(n?.children||[]).map(text).join('');
(async()=>{
 let tree;const Component=load('components/interview/BackgroundSettings.tsx').BackgroundSettings;
 const props={revision:'8',locked:false,names,onBusy(){},onApplied:async()=>{},applyNames:(expected,value)=>{namesCalls++;if(names!==expected)return false;names=value;return true;}};
 await act(async()=>{tree=create(React.createElement(Component,props));});
 const input=label=>tree.root.findByProps({'aria-label':label});
 const button=label=>tree.root.findAllByType('button').find(b=>text(b)===label);
 const click=async label=>{const b=button(label);assert(!b.props.disabled);await act(async()=>b.props.onClick());};
 const select=async(label,value=true)=>act(async()=>input('应用'+label).props.onChange({target:{checked:value}}));
 assert.equal(calls.length,1);assert(button('应用所选设置').props.disabled);assert(tree.root.findAllByType('input').every(i=>!i.props.checked));
 await select('岗位');await act(async()=>input('岗位建议').props.onChange({target:{value:'微调岗位'}}));
 gate=new Promise(r=>resolveGate=r);await act(async()=>{button('应用所选设置').props.onClick();button('应用所选设置').props.onClick();});
 assert.equal(calls.filter(c=>c.cmd==='background_settings_apply').length,1);
 assert.deepEqual(calls.at(-1).args,{version:'8',profileHash:'synthetic-hash',changes:[{field:'role',expected:'手填岗位',value:'微调岗位'}]});
 await act(async()=>resolveGate());gate=null;assert.equal(view.current.organization,'手填组织');assert.equal(namesCalls,0);assert(text(tree.toJSON()).includes('已应用所选设置'));
 await select('组织');await select('点名称呼');fail=true;await click('应用所选设置');assert.equal(namesCalls,0);assert(input('应用组织').props.checked);assert(text(tree.toJSON()).includes('已被手动修改'));
 fail=false;await select('组织',false);names='后来手动更改';await click('应用所选设置');assert.equal(names,'后来手动更改');assert.equal(calls.at(-1).args.changes.length,0);assert(text(tree.toJSON()).includes('已保留当前值'));
 await act(async()=>tree.update(React.createElement(Component,{...props,names,locked:true})));assert(button('重新读取差异').props.disabled);assert(input('岗位建议').props.disabled);
 await act(async()=>tree.unmount());
 // A late native completion cannot mutate reminder names after leaving panel.
 names='原称呼';await act(async()=>{tree=create(React.createElement(Component,props));});
 await select('点名称呼');gate=new Promise(r=>resolveGate=r);
 await act(async()=>button('应用所选设置').props.onClick());
 const beforeNames=namesCalls;await act(async()=>tree.unmount());
 await act(async()=>resolveGate());gate=null;assert.equal(namesCalls,beforeNames);
 // Real meeting hook preserves mode and refuses stale/failed local storage.
 const store=new Map([['tingxian-meeting-v1',JSON.stringify({mode:'mixed',names:'原称呼'})]]);let broken=false;
 global.localStorage={getItem:k=>store.get(k)||null,setItem:(k,v)=>{if(broken)throw Error('storage');store.set(k,v);}};
 const {useMeetingMode}=load('components/interview/MeetingMode.tsx');let meeting,cleared=0;
 function Host(){meeting=useMeetingMode(()=>cleared++);return null;}
 await act(async()=>{tree=create(React.createElement(Host));});
 await act(async()=>assert(meeting.applyNames('原称呼','小林')));assert.equal(meeting.setup.mode,'mixed');assert.equal(meeting.setup.names,'小林');assert.equal(cleared,1);
 await act(async()=>meeting.change({mode:'group',names:'手动名字'}));assert(!meeting.applyNames('小林','不能覆盖'));
 store.set('tingxian-meeting-v1',JSON.stringify({mode:'panel',names:'手动名字'}));assert(!meeting.applyNames('手动名字','不能覆盖模式'));
 store.set('tingxian-meeting-v1',JSON.stringify(meeting.setup));broken=true;assert(!meeting.applyNames('手动名字','不能冒称保存'));assert.equal(meeting.setup.names,'手动名字');
 await act(async()=>tree.unmount());
 console.log('PASS background settings: read-only preview, unchecked per-field edits, exact CAS payload, duplicate/session guards, stale values retained, names only after validation, mode/storage protection; no API/permission change');
})().catch(e=>{console.error(e);process.exitCode=1;});
