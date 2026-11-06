// Real review component: explicit confirmation, date errors, stale snapshots.
const fs=require('node:fs'), path=require('node:path'), assert=require('node:assert/strict');
const frontend=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=frontend('react'),{create,act}=frontend('react-test-renderer'),ts=frontend('typescript');
const calls=[],saves=[];let failPreview=false,failSave=false;
function load(relative) {
  const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src',relative),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX,esModuleInterop:true}}).outputText;
  const module={exports:{}};
  const req=name=>{
    if(name==='@/components/ui/button')return {Button:props=>React.createElement('button',props)};
    if(name==='@tauri-apps/api/core')return {invoke:async(name,args)=>{
      calls.push({name,args});assert.equal(name,'profile_document_segments');
      if(failPreview)throw Error('资料已更新，请刷新');
      return Array.from({length:12},(_,i)=>({location:'段落 '+(i+1),text:'合成内容 '+(i+1)}));
    }};
    if(name.startsWith('@/lib/'))return load(name.slice(2)+'.ts');
    if(name==='@/components/interview/BackgroundVersions')return load('components/interview/BackgroundVersions.tsx');
    return frontend(name);
  };
  new Function('require','module','exports',code)(req,module,module.exports);return module.exports;
}
const {DocumentReview}=load('components/interview/DocumentReview.tsx');
const text=node=>typeof node==='string'?node:(node?.children||[]).map(text).join('');
const document={path:'synthetic.md',root:'/synthetic/root',hash:'content-A',kind:'practice',allow_external:true,chunks:12,metadata:{confirmation:'unreviewed',validity:'unknown',confirmed_at:null,valid_from:null,valid_until:null,updated_at:null}};
const onSave=async update=>{saves.push(update);if(failSave)throw Error('资料根目录已改变，未保存');};
(async()=>{
  let tree;await act(async()=>{tree=create(React.createElement(DocumentReview,{document,locked:false,onSave}));});
  const button=label=>tree.root.findAllByType('button').find(n=>text(n)===label);
  const click=async label=>{assert.ok(!button(label).props.disabled);await act(async()=>{await button(label).props.onClick();});};
  const checkbox=()=>tree.root.findByProps({type:'checkbox'});
  const setDate=async(label,value)=>act(async()=>{tree.root.findByProps({'aria-label':'synthetic.md '+label}).props.onChange({target:{value}});});
  assert.equal(checkbox().props.checked,false);assert.equal(saves.length,0);
  await click('查看已导入片段');assert.deepEqual(calls[0],{name:'profile_document_segments',args:{path:document.path,expectedRoot:document.root,expectedHash:document.hash}});
  assert.ok(text(tree.toJSON()).includes('合成内容 12'),'review must show all imported segments');
  assert.equal(checkbox().props.checked,false,'preview alone must not confirm');
  await setDate('有效期开始','2026-11-03');await setDate('有效期结束','2026-11-02');
  await click('保存核对与日期');assert.equal(saves.length,0);assert.ok(text(tree.toJSON()).includes('结束日期不能早于'));
  await setDate('有效期结束','2026-11-03');await act(async()=>checkbox().props.onChange({target:{checked:true}}));
  await click('保存核对与日期');assert.deepEqual(saves[0],{path:document.path,expected_root:document.root,expected_hash:document.hash,confirmed:true,valid_from:'2026-11-03',valid_until:'2026-11-03'});
  failSave=true;await click('保存核对与日期');assert.ok(text(tree.toJSON()).includes('未保存'));
  failPreview=true;await click('查看已导入片段');assert.ok(text(tree.toJSON()).includes('资料已更新'));
  const changed={...document,hash:'content-B',metadata:{...document.metadata,confirmation:'content_changed',validity:'content_changed',confirmed_at:'2026-11-03T00:00:00.000Z'}};
  await act(async()=>tree.update(React.createElement(DocumentReview,{key:'changed',document:changed,locked:true,onSave})));
  assert.equal(checkbox().props.checked,false);assert.ok(text(tree.toJSON()).includes('内容已变化'));
  assert.ok(tree.root.findAllByType('input').every(n=>n.props.disabled));assert.ok(tree.root.findAllByType('button').every(n=>n.props.disabled));
  await act(async()=>tree.unmount());
  console.log('PASS: explicit review, all snapshot segments, content/root identity, date range validation, surfaced failures, changed-content and session locks');
})().catch(error=>{console.error(error);process.exitCode=1;});
