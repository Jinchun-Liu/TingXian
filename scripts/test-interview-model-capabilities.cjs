// Real settings components against isolated IPC; no credentials or model calls.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=req('react'),{act,create}=req('react-test-renderer'),ts=req('typescript');
const request={model:'synthetic',connect_timeout_secs:5,first_answer_warning_secs:8,total_timeout_secs:20};
const profile=id=>({id,name:id,enabled:id==='answer-1',endpoint:'https://synthetic.invalid/chat/completions',protocol:'openai',request:{...request}});
let stored={answers:[1,2,3].map(i=>profile(`answer-${i}`)),vision:profile('vision'),background:profile('background')};const calls=[];
const invoke=async(cmd,args)=>{calls.push({cmd,args});if(cmd==='interview_providers_get')return {config:JSON.parse(JSON.stringify(stored)),capacity:0,key_configured:{}};
 if(cmd==='interview_providers_save'){stored=JSON.parse(JSON.stringify(args.config));return;}
 if(cmd==='interview_provider_test')return {success:true,message:'固定文字连接成功，不能验证图片或窗口',elapsed_ms:10};
 throw Error('Unexpected command '+cmd);
};
const modules={};function load(name){if(modules[name])return modules[name].exports;const m=modules[name]={exports:{}};
 const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview',name+'.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX}}).outputText;
 new Function('require','module','exports',code)(n=>n==='@tauri-apps/api/core'?{invoke}:n==='@/components/ui/button'?{Button:'button'}:n==='@/components/ui/choice-menu'?{ChoiceMenu:'select',ChoiceOption:'option'}:n.startsWith('@/components/interview/')?load(n.split('/').at(-1)):n.startsWith('./')?load(n.slice(2)):req(n),m,m.exports);return m.exports;
}
const text=n=>typeof n==='string'?n:(n?.children||[]).map(text).join('');
(async()=>{
 const {ProviderSettings}=load('ProviderSettings');const {capabilitiesError}=load('ModelCapabilities');let tree,locked=false;
 const render=()=>{const el=React.createElement(ProviderSettings,{locked,onSaved:async()=>{},onBusyChange:()=>{}});tree?tree.update(el):tree=create(el);};await act(async()=>render());
 const input=label=>tree.root.findByProps({'aria-label':label});
 const edit=async(label,value)=>act(async()=>input(label).props.onChange({target:{value}}));
 const choose=async(label,value)=>act(async()=>input(label).props.onValueChange(value));
 const click=async label=>{const b=tree.root.findAllByType('button').find(n=>text(n).startsWith(label));assert(b&&!b.props.disabled,label);await act(async()=>b.props.onClick());};
 const saves=()=>calls.filter(c=>c.cmd==='interview_providers_save').length;
 const review=async id=>{await choose(id+' 文字输入','supported');await choose(id+' 图片输入',id==='vision'?'supported':'unsupported');await choose(id+' 流式响应','supported');await choose(id+' 返回用量','unknown');await edit(id+' 能力核对日期','2020-01-02');await edit(id+' 能力核对来源','虚构规格，不是实测能力');};
 assert.equal(saves(),0);assert.equal(input('answer-1 图片输入').props.value,'unknown');
 await choose('answer-1 图片输入','unsupported');await click('保存多接口配置');assert.equal(saves(),0);assert(text(tree.toJSON()).includes('核对日期'));
 await review('answer-1');await review('vision');await click('保存多接口配置');assert.equal(saves(),1);assert.equal(stored.answers[0].request.capabilities.image,'unsupported');assert.equal(stored.vision.request.capabilities.image,'supported');
 const original={...stored.answers[0].request.capabilities};await edit('answer-1 安全余量','1024');await click('保存多接口配置');assert.deepEqual(stored.answers[0].request.capabilities,original,'safety edit discarded capability review');
 await click('测试已保存接口');assert.deepEqual(stored.answers[0].request.capabilities,original,'text probe certified or rewrote capabilities');assert.equal(stored.answers[1].request.capabilities,undefined);
 await edit('answer-1 模型 ID','changed-model');assert.equal(input('answer-1 图片输入').props.value,'unknown');await click('保存多接口配置');assert.equal(stored.answers[0].request.capabilities,undefined);assert.equal(stored.vision.request.capabilities.image,'supported');
 await review('answer-1');await edit('answer-1 API 地址','https://changed.invalid/chat/completions');assert.equal(input('answer-1 流式响应').props.value,'unknown');
 await review('answer-1');await edit('answer-1 模型窗口','16384');assert.equal(input('answer-1 图片输入').props.value,'unknown');
 await review('answer-1');await edit('answer-1 模型输出上限','1024');assert.equal(input('answer-1 图片输入').props.value,'unknown');
 await review('answer-1');await edit('answer-1 能力核对日期','9999-01-01');const before=saves();await click('保存多接口配置');assert.equal(saves(),before);
 await edit('answer-1 能力核对日期','2020-02-30');await click('保存多接口配置');assert.equal(saves(),before);
 await click('清除能力记录');await click('保存多接口配置');assert.equal(stored.answers[0].request.capabilities,undefined);
 assert(capabilitiesError({...original,model:'old'},'different',{context_tokens:262144,max_output_tokens:8192},original.endpoint));
 locked=true;await act(async()=>render());assert(tree.root.findAllByType('input').every(i=>i.props.disabled));assert(tree.root.findAllByType('select').every(i=>i.props.disabled));
 assert(calls.every(c=>['interview_providers_get','interview_providers_save','interview_provider_test'].includes(c.cmd)));await act(async()=>tree.unmount());
 console.log('PASS model capabilities: legacy unknown, required manual source/date, slot isolation, changed identity/limits invalidate review, safety edits preserve, text probe never certifies, clear and session locks');
})().catch(e=>{console.error(e);process.exitCode=1;});
