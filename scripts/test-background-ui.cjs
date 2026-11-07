// Verify that selecting documents cannot transmit them; review and apply are distinct.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const dep=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=dep('react'),{act,create}=dep('react-test-renderer'),ts=dep('typescript');
const calls=[];const p={overview:'虚构材料，模拟项目',role:'示例岗位',aliases:'模拟,mock'};
let resumeFail=false,trashFail=false;
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
  if(cmd==='background_job_details')return [{name:'中断文件.md',status:'processing',done:2,total:4,reason:''}];
  if(cmd==='background_resume'){if(resumeFail)throw Error('当前背景模型与断点不一致');view.work.draft={id:'resumed',job:args.job,profile:p,documents:[{id:'file',name:'新材料.md',count:1}],failures:[],metrics:{calls:2,peak_prompt_tokens:100}};return;}
  if(cmd==='background_job_trash'){if(trashFail)throw Error('移入废纸篓失败');view.jobs=[];if(view.work.draft?.job===args.job)view.work.draft=null;return;}
  if(cmd==='background_apply'){assert.equal(args.profile.role,'核对后岗位');assert.equal(args.share,false);assert.equal(args.mergeTerms,true);view.library.version='8';view.library.state={profile:args.profile,share:false};view.work.draft=null;return;}
  throw Error(cmd);
};
const mod={exports:{}};
const code=ts.transpileModule(fs.readFileSync(path.join(__dirname,'../frontend/src/components/interview/BackgroundLibrary.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX}}).outputText;
new Function('require','module','exports',code)(name=>name==='@tauri-apps/api/core'?{invoke}:name==='@/components/interview/PreparationLibrary'?{PreparationLibrary:()=>null}:name==='@/components/interview/BackgroundSettings'?{BackgroundSettings:()=>null}:name==='@/components/interview/BackgroundChanges'?{BackgroundChanges:()=>null}:name==='@/components/ui/button'?{Button:props=>React.createElement('button',props)}:dep(name),mod,mod.exports);
const text=n=>typeof n==='string'?n:(n?.children||[]).map(text).join('');
(async()=>{let t;await act(async()=>{t=create(React.createElement(mod.exports.BackgroundLibrary,{active:true,revision:'7',locked:false,onBusy(){},onApplied:async()=>{},onModels(){}}))});
  const click=async label=>{const b=t.root.findAllByType('button').find(x=>text(x)===label);assert.ok(b,label);assert.ok(!b.props.disabled);await act(async()=>b.props.onClick());};
  await click('添加重点文件');assert.deepEqual(calls.map(c=>c[0]),['background_status','background_choose']);
  await click('发送并整理');assert(!calls.some(c=>c[0]==='background_apply'));
  await click('新材料.md · 1 条引文');assert(text(t.toJSON()).includes('原文'));
  await act(async()=>t.root.findAllByType('textarea')[1].props.onChange({target:{value:'核对后岗位'}}));
  await click('应用到资料库');await click('用背景模型计量可外发资料');assert(text(t.toJSON()).includes('123 输入 tokens'));
  await act(async()=>t.unmount());
  view.jobs=[{id:'synthetic-job',model:'synthetic',status:'interrupted',total:2,completed:1,failed:0,skipped:0,unknown_calls:1,metrics:{calls:3,sum_prompt_tokens:123,sum_completion_tokens:8192,peak_prompt_tokens:100}}];
  const before=calls.filter(c=>c[0]==='background_resume').length;
  await act(async()=>{t=create(React.createElement(mod.exports.BackgroundLibrary,{active:true,revision:'8',locked:false,onBusy(){},onApplied:async()=>{},onModels(){}}))});
  assert.equal(calls.filter(c=>c[0]==='background_resume').length,before,'opening app resumed external work');assert(text(t.toJSON()).includes('用量未知'));
  assert(text(t.toJSON()).includes('请求 3 次 · 已知输入 123 / 输出 8,192 tokens'));
  assert(!text(t.toJSON()).includes('次中断请求'),'missing provider usage was mislabeled as interrupted');
  await click('查看进度');assert(text(t.toJSON()).includes('中断文件.md'));assert(text(t.toJSON()).includes('2/4 段'));
  resumeFail=true;await click('继续并核对');assert(text(t.toJSON()).includes('当前背景模型与断点不一致'));assert.equal(view.work.draft,null);
  resumeFail=false;await click('继续并核对');assert(text(t.toJSON()).includes('核对整理结果'));assert.equal(calls.filter(c=>c[0]==='background_apply').length,1,'resume applied a draft without review');
  trashFail=true;await click('移入废纸篓');assert.equal(view.jobs.length,1);assert(text(t.toJSON()).includes('移入废纸篓失败'));
  trashFail=false;await click('移入废纸篓');assert.equal(view.jobs.length,0);assert(!text(t.toJSON()).includes('核对整理结果'));
  await act(async()=>t.unmount());
  console.log('PASS: explicit send, source preview, editable atomic application, local-only default and metered tokens');
  console.log('PASS: restart discovers durable tasks without upload; explicit resume, model-change error, per-file progress, unknown usage, review-before-apply and recoverable removal');
})().catch(e=>{console.error(e);process.exitCode=1;});
