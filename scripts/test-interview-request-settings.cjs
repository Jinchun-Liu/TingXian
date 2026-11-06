// Actual settings component; synthetic native boundary and asynchronous races.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const frontend=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=frontend('react'),{act,create}=frontend('react-test-renderer'),ts=frontend('typescript');
let stored={model:'synthetic-model',connect_timeout_secs:2,first_answer_warning_secs:3,total_timeout_secs:10};
let failSave=false,failRead=false,pendingRead=null,locked=false;const calls=[];
const invoke=async(name,args)=>{
 calls.push({name,args});
 if(name==='interview_request_config_get'){if(failRead)throw Error('synthetic read failure');if(pendingRead)return new Promise(resolve=>{pendingRead=resolve;});return {...stored};}
 assert.equal(name,'interview_request_config_save');if(failSave)throw Error('synthetic save failure');stored={...args.config};return {...stored};
};
const caps={exports:{}};
new Function('require','module','exports',ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/ModelCapabilities.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX}}).outputText)(n=>n==='@/components/ui/choice-menu'?{ChoiceMenu:'select',ChoiceOption:'option'}:n==='@/components/ui/button'?{Button:'button'}:frontend(n),caps,caps.exports);
const moduleObject={exports:{}};
const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/RequestSettings.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX,esModuleInterop:true}}).outputText;
new Function('require','module','exports',code)(name=>name==='@/components/interview/ModelCapabilities'?caps.exports:name==='@tauri-apps/api/core'?{invoke}:name==='@/components/ui/button'?{Button:props=>React.createElement('button',props)}:frontend(name),moduleObject,moduleObject.exports);
const {RequestSettings}=moduleObject.exports;
const text=node=>typeof node==='string'?node:(node?.children||[]).map(text).join('');
(async()=>{
 let tree;const render=()=>{const element=React.createElement(RequestSettings,{locked});if(tree)tree.update(element);else tree=create(element);};
 await act(async()=>render());
 const input=label=>tree.root.findByProps({'aria-label':label});
 const button=label=>tree.root.findAllByType('button').find(n=>text(n)===label);
 const click=async label=>{const b=button(label);assert.ok(b);assert.ok(!b.props.disabled);await act(async()=>b.props.onClick());};
 const edit=async(label,value)=>act(async()=>input(label).props.onChange({target:{value}}));
 const saveCount=()=>calls.filter(c=>c.name==='interview_request_config_save').length;
 assert.equal(input('回答模型 ID').props.value,'synthetic-model');assert.equal(saveCount(),0);
 await edit('连接超时（秒）','');await click('保存回答请求设置');assert.equal(saveCount(),0);assert.ok(text(tree.toJSON()).includes('整数秒'));
 await edit('连接超时（秒）','2');await edit('未收到建议的提示时间（秒）','10');await click('保存回答请求设置');assert.equal(saveCount(),0);
 await edit('未收到建议的提示时间（秒）','4');failSave=true;await click('保存回答请求设置');assert.ok(text(tree.toJSON()).includes('synthetic save failure'));assert.equal(input('未收到建议的提示时间（秒）').props.value,'4');
 failSave=false;await click('保存回答请求设置');assert.equal(stored.first_answer_warning_secs,4);assert.equal(typeof stored.first_answer_warning_secs,'number');assert.ok(text(tree.toJSON()).includes('下次回答'));
 await edit('回答 模型窗口','16384');await edit('回答 模型输出上限','1024');await edit('回答 安全余量','512');await click('保存回答请求设置');assert.deepEqual(stored.budget,{context_tokens:16384,max_output_tokens:1024,safety_tokens:512});
 await edit('未收到建议的提示时间（秒）','5');await click('保存回答请求设置');assert.equal(stored.budget.context_tokens,16384,'editing timeout reset the model budget');
 const budgetSaves=saveCount();await edit('回答 模型输出上限','16384');await click('保存回答请求设置');assert.equal(saveCount(),budgetSaves);assert.ok(text(tree.toJSON()).includes('检查模型预算'));await edit('回答 模型输出上限','1024');
 locked=true;await act(async()=>render());assert.ok(input('回答模型 ID').props.disabled);assert.ok(button('恢复默认请求设置').props.disabled);
 locked=false;await act(async()=>render());const count=saveCount();await click('恢复默认请求设置');assert.equal(saveCount(),count);assert.equal(input('单次总截止（秒）').props.value,'20');await click('保存回答请求设置');assert.equal(stored.model,'deepseek-flash');
 failRead=true;await click('重新读取请求设置');assert.ok(text(tree.toJSON()).includes('synthetic read failure'));assert.ok(button('保存回答请求设置').props.disabled);
 await click('恢复默认请求设置');assert.ok(!button('保存回答请求设置').props.disabled);failRead=false;
 pendingRead=true;await click('重新读取请求设置');assert.equal(typeof pendingRead,'function');await click('恢复默认请求设置');
 await act(async()=>{const resolve=pendingRead;pendingRead=null;resolve({model:'late-model',connect_timeout_secs:1,first_answer_warning_secs:2,total_timeout_secs:6});});
 assert.equal(input('回答模型 ID').props.value,'deepseek-flash','late read overwrote deliberate reset');
 assert.ok(calls.every(c=>['interview_request_config_get','interview_request_config_save'].includes(c.name)),'settings must not start capture or model requests');
 await act(async()=>tree.unmount());
 console.log('PASS: actual request settings load/edit/save, numeric validation, failure draft retention, session lock, explicit default/reset, stale read protection and zero model/capture commands');
})().catch(error=>{console.error(error);process.exitCode=1;});
