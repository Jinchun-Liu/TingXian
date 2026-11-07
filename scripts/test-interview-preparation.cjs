const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=req('react'),{act,create}=req('react-test-renderer'),ts=req('typescript');
const calls=[],busy=[];let gate=null,resolveGate,error=null,cancel=false;
const candidate={candidate_hash:'candidate',execution_mode:'fixture',task:{role:'测试开发',organization:'合成组织'},
 role_view:[{id:'req-1',text:'契约测试',matches:[{record_id:'r1',relevance:'supporting',reason:'只能证明本地测试'}],references:[],gaps:[{text:'生产经验待补'}]}],
 source_records:[{record_id:'r1',claim:'计划补充测试，未完成',claim_status:'user_asserted',activity_status:'planned',manual_revision:{basis:'本人核对'},source_refs:[{source_id:'source',locator:'行1',quote:'旧原文，尚未核实'}]}],
 stories:Array.from({length:11},(_,i)=>({id:'story'+i,title:'故事'+(i+1),activity:'planned',sections:{context:[],task:[],action:[{text:'计划补充测试，未完成',record_ids:['r1']}],result:[],learning:[]},missing:{context:'上下文',task:'职责',result:'验证',learning:'反思'}}))};
const preview={candidate,index_version:'7',previous_hash:'saved-hash',source_binding_valid:true};
const invoke=async(command,args)=>{
 calls.push({command,args});const held=gate;if(held)await held;if(error)throw Error(error);
 if(command==='background_preparation_import')return cancel?null:structuredClone(preview);
 if(command==='background_preparation_view')return {candidate:structuredClone(candidate),source_binding_valid:false,local_only:true};
 if(command==='background_preparation_save')return;
 if(command==='background_preparation_export')return '/synthetic/preparation.json';
 throw Error('Unexpected command '+command);
};
const mod={exports:{}};
const source=fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/PreparationLibrary.tsx'),'utf8');
new Function('require','module','exports',ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX}}).outputText)(n=>n==='@tauri-apps/api/core'?{invoke}:n==='@/components/ui/button'?{Button:'button'}:req(n),mod,mod.exports);
const text=n=>typeof n==='string'?n:(n?.children||[]).map(text).join('');
(async()=>{
 let tree;const props={locked:false,revision:'7',onBusy:v=>busy.push(v)},Component=mod.exports.PreparationLibrary;
 const button=label=>tree.root.findAllByType('button').find(n=>text(n)===label);
 const click=async label=>{const b=button(label);assert(b&&!b.props.disabled,label);await act(async()=>{await b.props.onClick();});};
 await act(async()=>{tree=create(React.createElement(Component,props));});assert.equal(calls.length,0,'mount should not export/read user material');
 await click('导出整理资料');assert(text(tree.toJSON()).includes('已导出'));assert.equal(calls.length,1);
 cancel=true;await click('导入准备候选');assert(!button('保存岗位准备'));cancel=false;
 await click('导入准备候选');assert(text(tree.toJSON()).includes('本人修订：计划'));assert(text(tree.toJSON()).includes('旧原文'));assert(text(tree.toJSON()).includes('生产经验待补'));assert(text(tree.toJSON()).includes('故事10'));assert(!text(tree.toJSON()).includes('故事11'));
 await click('下一页故事');assert(text(tree.toJSON()).includes('故事11'));await click('上一页故事');
 error='资料库已变化';await click('保存岗位准备');assert(text(tree.toJSON()).includes('资料库已变化'));assert(!text(tree.toJSON()).includes('已保存岗位准备'));assert(button('保存岗位准备'));error=null;
 await click('保存岗位准备');assert.deepEqual(calls.at(-1).args,{candidate,version:'7',previousHash:'saved-hash'});assert(!button('保存岗位准备'));assert(text(tree.toJSON()).includes('个人事实未改变'));
 await click('查看已保存');assert(text(tree.toJSON()).includes('来源已变化'));assert(!button('保存岗位准备'),'stale history is not a new draft');
 gate=new Promise(r=>resolveGate=r);await act(async()=>{button('导入准备候选').props.onClick();button('导入准备候选').props.onClick();});const count=calls.length;
 await act(async()=>tree.update(React.createElement(Component,{...props,revision:'8'})));gate=null;await act(async()=>resolveGate());assert.equal(calls.length,count);assert(!button('保存岗位准备'),'old source import returned after refresh');
 await click('导入准备候选');await click('取消导入');assert(!button('保存岗位准备'));
 await act(async()=>tree.update(React.createElement(Component,{...props,locked:true,revision:'8'})));assert(tree.root.findAllByType('button').every(b=>b.props.disabled));
 await act(async()=>tree.unmount());assert.equal(busy.at(-1),false);
 assert(calls.every(c=>c.command.startsWith('background_preparation_')),'candidate changed facts, providers or runtime');
 console.log('PASS preparation UI: explicit export/import, original and manual quotes, gaps, paging, save CAS, stale import/history, session lock, no fact/model commands');
})().catch(e=>{console.error(e);process.exitCode=1;});
