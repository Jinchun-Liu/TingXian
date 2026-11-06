const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=req('react'),{act,create}=req('react-test-renderer'),ts=req('typescript');
let gate=null,resolveGate,fail=false;const calls=[];
const invoke=async(cmd,args)=>{
 calls.push({cmd,args});if(gate)await gate;if(fail)throw Error('来源已变化，请刷新');
 if(cmd==='background_changes')return [{id:args.draft,name:args.draft,state:'source_changed',identical_count:2,identical_sources:['副本一','副本二']}];
 if(cmd==='background_versions')return {rows:[{revision:'old',name:'虚构材料',source_hash:'1234567890123456789',recorded_at:null,count:21,current:false}],more:false};
 if(cmd==='background_version_records')return {rows:[{location:'行 1',quote:args.offset?'第二页旧原文':'原文：仅在本地模拟',note:'不是生产经验',kind:'practice'}],more:args.offset===0,current:false};
 throw Error('Unexpected mutating/model command '+cmd);
};
function load(name){const m={exports:{}};const source=fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview',name+'.tsx'),'utf8');
 new Function('require','module','exports',ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX}}).outputText)(n=>n==='@tauri-apps/api/core'?{invoke}:n==='@/components/ui/button'?{Button:'button'}:req(n),m,m.exports);return m.exports[name];}
const text=n=>typeof n==='string'?n:(n?.children||[]).map(text).join('');
(async()=>{
 let tree;const Changes=load('BackgroundChanges'),Versions=load('BackgroundVersions');
 const button=label=>tree.root.findAllByType('button').find(b=>text(b).includes(label));
 const click=async label=>{const b=button(label);assert(b&&!b.props.disabled,label);await act(async()=>{await b.props.onClick();});};
 gate=new Promise(r=>resolveGate=r);await act(async()=>{tree=create(React.createElement(Changes,{draft:'old-draft'}));});
 gate=null;await act(async()=>tree.update(React.createElement(Changes,{draft:'new-draft'})));await act(async()=>resolveGate());
 assert(!text(tree.toJSON()).includes('old-draft'));assert(text(tree.toJSON()).includes('new-draft'));assert(text(tree.toJSON()).includes('不是额外的独立事实证明'));
 await act(async()=>tree.unmount());
 const before=calls.length;const props={source:'synthetic',expected:'hashA',locked:false};
 await act(async()=>{tree=create(React.createElement(Versions,props));});assert.equal(calls.length,before,'mount read history without explicit request');
 await click('查看来源版本');assert(text(tree.toJSON()).includes('早期导入，时间未知'));assert(text(tree.toJSON()).includes('不进入当前问答'));
 await click('历史 ·');assert(text(tree.toJSON()).includes('仅在本地模拟'));await click('下一页引文');assert.equal(calls.at(-1).args.offset,20);assert(text(tree.toJSON()).includes('第二页旧原文'));
 fail=true;await click('上一页引文');assert(text(tree.toJSON()).includes('来源已变化'));assert(!text(tree.toJSON()).includes('第二页旧原文'));fail=false;
 gate=new Promise(r=>resolveGate=r);await act(async()=>{button('查看来源版本').props.onClick();button('查看来源版本').props.onClick();});
 const pending=calls.length;await act(async()=>tree.update(React.createElement(Versions,{...props,expected:'hashB'})));
 assert(!button('查看来源版本').props.disabled,'stale read left panel permanently busy');
 gate=null;await click('查看来源版本');assert.equal(calls.length,pending+1);assert.equal(calls.at(-1).args.expected,'hashB');
 await act(async()=>resolveGate());await act(async()=>tree.update(React.createElement(Versions,{...props,expected:'hashB',locked:true})));
 assert(tree.root.findAllByType('button').every(b=>b.props.disabled));await act(async()=>tree.unmount());
 console.log('PASS background provenance: stale draft/read isolation, explicit paginated history, unknown legacy time, immutable old excerpts, duplicate guard, changed source recovery and session locks; read-only IPC');
})().catch(e=>{console.error(e);process.exitCode=1;});
