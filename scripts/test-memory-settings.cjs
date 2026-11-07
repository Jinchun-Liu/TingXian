const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json')),ts=req('typescript'),React=req('react'),{create,act}=req('react-test-renderer');
let stored=require('./test-support/memory-disabled.cjs')('interview_memory_config_get'),calls=[],locked=false;
const invoke=async(cmd,args)=>{calls.push({cmd,args});if(cmd==='interview_memory_config_get')return structuredClone(stored);if(cmd==='interview_memory_config_save'){stored={config:structuredClone(args.config),key_configured:!!args.key};return;}if(cmd==='interview_memory_test')return {message:'仅合成连接结果',elapsed_ms:5,total_tokens:null};throw Error(cmd);};
const cache={};function load(name){if(cache[name])return cache[name];const m={exports:{}};const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/'+name+'.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX}}).outputText;new Function('require','module','exports',code)(n=>n==='@tauri-apps/api/core'?{invoke}:n==='@tauri-apps/api/event'?{listen:async()=>()=>{}}:n==='@/components/ui/button'?{Button:'button'}:n==='@/components/ui/choice-menu'?{ChoiceMenu:'select',ChoiceOption:'option'}:n.startsWith('@/components/interview/')?load(n.split('/').at(-1)):req(n),m,m.exports);return cache[name]=m.exports;}
const text=n=>typeof n==='string'?n:(n?.children||[]).map(text).join('');
(async()=>{
 const {MemorySettings}=load('MemorySettings');let tree;const render=()=>{const el=React.createElement(MemorySettings,{locked,onSaved:async()=>{}});tree?tree.update(el):tree=create(el);};await act(async()=>render());
 const input=l=>tree.root.findByProps({'aria-label':l});const button=l=>tree.root.findAllByType('button').find(b=>text(b).startsWith(l));
 const edit=async(l,v)=>act(async()=>input(l).props.onChange({target:{value:v}}));
 assert(!tree.root.findByProps({type:'checkbox'}).props.checked);assert.deepEqual(calls.map(c=>c.cmd),['interview_memory_config_get']);
 await act(async()=>tree.root.findByProps({type:'checkbox'}).props.onChange({target:{checked:true}}));await edit('压缩 API Key','synthetic-key');await act(async()=>button('保存会话记忆设置').props.onClick());
 assert.equal(stored.config.enabled,true);assert.equal(input('压缩 API Key').props.value,'');assert.equal(calls.filter(c=>c.cmd==='interview_memory_test').length,0);
 await edit('压缩 模型窗口','4096');await act(async()=>button('保存会话记忆设置').props.onClick());assert(text(tree.toJSON()).includes('预算'));assert.equal(stored.config.profile.request.budget.context_tokens,262144);
 await act(async()=>button('测试已保存压缩接口').props.onClick());assert(text(tree.toJSON()).includes('用量未知'));assert.equal(calls.filter(c=>c.cmd==='interview_memory_test').length,1);
 locked=true;await act(async()=>render());assert(tree.root.findAllByType('input').every(i=>i.props.disabled));assert(button('保存会话记忆设置').props.disabled);assert(button('测试已保存压缩接口').props.disabled);await act(async()=>tree.unmount());
 console.log('PASS: independent default-off UI, native-only key draft, no implicit test on save, actual output budget check, explicit metered connection test, locked session');
})().catch(e=>{console.error(e);process.exitCode=1;});
