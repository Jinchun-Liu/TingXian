const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=req('react'),{act,create}=req('react-test-renderer'),ts=req('typescript');
let config={mode:'local',workspace_id:'',silence_ms:1300},configured=false,fail=false,gate=null,resolveGate,saved=0;const calls=[];
const invoke=async(cmd,args)=>{
 calls.push({cmd,args});if(cmd==='interview_cloud_asr_get')return {config:{...config},key_configured:configured};
 assert.equal(cmd,'interview_cloud_asr_save');if(gate)await gate;if(fail)throw Error('storage failed');config={...args.config};if(args.key!==null)configured=!!args.key;return;
};
const source=fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/CloudAsrSettings.tsx'),'utf8'),mod={exports:{}};
new Function('require','module','exports',ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX}}).outputText)(n=>{
 if(n==='@tauri-apps/api/core')return {invoke};if(n==='@/components/ui/button')return {Button:'button'};if(n==='@/components/ui/choice-menu')return {ChoiceMenu:'choice',ChoiceOption:'option'};return req(n);
},mod,mod.exports);
(async()=>{
 let tree;const render=locked=>React.createElement(mod.exports.CloudAsrSettings,{locked,onSaved:()=>saved++});
 await act(async()=>{tree=create(render(false));});
 assert.equal(calls.length,1,'opening settings initiated an audio/model request');
 const field=label=>tree.root.findByProps({'aria-label':label});const button=()=>tree.root.findByType('button');
 await act(async()=>field('转写服务').props.onValueChange('aliyun'));assert.equal(button().props.disabled,true);
 await act(async()=>{field('百炼 Workspace ID').props.onChange({target:{value:'llm-synthetic'}});field('云转写 API Key').props.onChange({target:{value:'synthetic-only'}});});
 fail=true;await act(async()=>button().props.onClick());assert.equal(field('云转写 API Key').props.value,'synthetic-only');assert.equal(saved,0);
 fail=false;gate=new Promise(r=>resolveGate=r);await act(async()=>{button().props.onClick();button().props.onClick();});assert.equal(calls.filter(c=>c.cmd.endsWith('_save')).length,2,'duplicate submit bypassed busy guard');
 await act(async()=>resolveGate());gate=null;assert.equal(saved,1);assert.equal(field('云转写 API Key').props.value,'');assert.equal(config.mode,'aliyun');
 await act(async()=>tree.update(render(true)));assert.equal(button().props.disabled,true);
 await act(async()=>button().props.onClick());assert.equal(saved,1,'locked session changed provider');
 await act(async()=>{tree.update(render(false));});await act(async()=>field('转写服务').props.onValueChange('local'));
 await act(async()=>button().props.onClick());assert.equal(config.mode,'local');assert.equal(calls.at(-1).args.key,null,'switching local cleared an unchanged credential');
 await act(async()=>tree.unmount());
 console.log('PASS cloud ASR settings: local default, explicit save, workspace/empty gates, credential clearing after success, failure retention, duplicate and active-session guards; no capture/API request');
})().catch(e=>{console.error(e);process.exitCode=1;});
