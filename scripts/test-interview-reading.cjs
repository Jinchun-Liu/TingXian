const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=req('react'),{create,act}=req('react-test-renderer'),ts=req('typescript');
const mod={exports:{}};
const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/ConversationFeed.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX}}).outputText;
new Function('require','module','exports',code)(req,mod,mod.exports);
(async()=>{
  let tree,end=1200,latest,search=false;
  const region={scrollTop:0,clientTop:0,clientHeight:400,getBoundingClientRect:()=>({top:0}),querySelector:()=>({
    getBoundingClientRect:()=>({bottom:end-region.scrollTop,top:end-region.scrollTop-60,height:60})
  })};
  const render=()=>React.createElement(mod.exports.ConversationFeed,{latest,readingHistory:search,onReturn:()=>{search=false;}},'history');
  const update=()=>act(async()=>tree.update(render()));
  await act(async()=>{tree=create(render(),{createNodeMock:el=>el.props.role==='region'?region:null});});
  assert.equal(region.scrollTop,0);assert.equal(tree.root.findAllByType('button').length,0);
  latest={id:'a',text:'first'};await update();assert.equal(region.scrollTop,800);
  end+=100;latest={...latest};await update();assert.equal(region.scrollTop,800,'a transcript-only render scrolled');
  latest.text+=' chunk';await update();assert.equal(region.scrollTop,900);
  for(const deltaY of [-20,20]){
    await act(async()=>tree.root.findByProps({role:'region'}).props.onWheel({deltaY}));
    region.scrollTop=300;end+=100;latest={id:'b',text:'next answer'};await update();
    assert.equal(region.scrollTop,300,'manual reading was displaced');
    await act(async()=>tree.root.findByType('button').props.onClick());assert.equal(region.scrollTop,end-400);
    end+=80;latest.text+=' more';await update();assert.equal(region.scrollTop,end-400);
  }
  search=true;await update();region.scrollTop=200;end+=90;latest.text+=' more';await update();assert.equal(region.scrollTop,200);
  await act(async()=>tree.root.findByType('button').props.onClick());await update();assert.equal(region.scrollTop,end-400);
  await act(async()=>tree.unmount());console.log('PASS: follows answer text only, wheel in both directions locks reading, return resumes, search is preserved');
})().catch(e=>{console.error(e);process.exitCode=1;});
