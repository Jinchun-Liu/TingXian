// Verify that selecting documents cannot transmit them; review and apply are distinct.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const dep=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=dep('react'),{act,create}=dep('react-test-renderer'),ts=dep('typescript');
const calls=[];const p={overview:'虚构材料，模拟项目',role:'示例岗位',aliases:'模拟,mock'};
const view={ready:true,model:'synthetic',work:{busy:false,draft:null},library:{documents:3,chunks:35,baseline:262144,full_budget:123776,rag_budget:54144,fits_full:true,version:'7',state:null,measurement:null}};
const invoke=async(cmd,args)=>{
  calls.push([cmd,args]);
  if(cmd==='background_status')return structuredClone(view);
  if(cmd==='background_choose')return {id:'selection',label:'新材料.md',count:1,folder:false};
  if(cmd==='background_generate'){
    if(args.selection==='measure'){view.library.measurement={model:'synthetic',version:'8',chunks:2,metrics:{calls:1,sum_prompt_tokens:123}};return;}
    assert.equal(args.includeLibrary,true);
    view.work.draft={id:'draft1',profile:p,documents:[{id:'file',name:'新材料.md',count:1}],failures:[],metrics:{calls:2,peak_prompt_tokens:100}};return;
  }
  if(cmd==='background_preview')return {rows:[{location:'行 1',quote:'原文',note:'练习'}],more:false};
  if(cmd==='background_apply'){assert.equal(args.profile.role,'核对后岗位');assert.equal(args.share,false);assert.equal(args.mergeTerms,true);view.library.version='8';view.library.state={profile:args.profile,share:false};view.work.draft=null;return;}
  throw Error(cmd);
};
const mod={exports:{}};
const code=ts.transpileModule(fs.readFileSync(path.join(__dirname,'../frontend/src/components/interview/BackgroundLibrary.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX}}).outputText;
new Function('require','module','exports',code)(name=>name==='@tauri-apps/api/core'?{invoke}:name==='@/components/ui/button'?{Button:props=>React.createElement('button',props)}:dep(name),mod,mod.exports);
const text=n=>typeof n==='string'?n:(n?.children||[]).map(text).join('');
(async()=>{let t;await act(async()=>{t=create(React.createElement(mod.exports.BackgroundLibrary,{active:true,revision:'7',locked:false,onBusy(){},onApplied:async()=>{},onModels(){}}))});
  const click=async label=>{const b=t.root.findAllByType('button').find(x=>text(x)===label);assert.ok(b,label);assert.ok(!b.props.disabled);await act(async()=>b.props.onClick());};
  await click('添加重点文件');assert.deepEqual(calls.map(c=>c[0]),['background_status','background_choose']);
  await click('发送并整理');assert(!calls.some(c=>c[0]==='background_apply'));
  await click('新材料.md · 1 条引文');assert(text(t.toJSON()).includes('原文'));
  await act(async()=>t.root.findAllByType('textarea')[1].props.onChange({target:{value:'核对后岗位'}}));
  await click('应用到资料库');await click('用背景模型计量可外发资料');assert(text(t.toJSON()).includes('123 输入 tokens'));
  console.log('PASS: explicit send, source preview, editable atomic application, local-only default and metered tokens');
})().catch(e=>{console.error(e);process.exitCode=1;});
