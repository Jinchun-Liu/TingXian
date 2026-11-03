const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const fromFrontend=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const React=fromFrontend('react'),{create,act}=fromFrontend('react-test-renderer'),ts=fromFrontend('typescript');
const m={exports:{}};
const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/ConversationFeed.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX}}).outputText;
new Function('require','module','exports',code)(name=>name==='@/components/ui/button'?{Button:p=>React.createElement('button',p)}:fromFrontend(name),m,m.exports);
const {ConversationFeed}=m.exports;
const text=node=>typeof node==='string'?node:(node?.children||[]).map(text).join('');
(async()=>{
 let tree,revision=0,generation=0,top=100,scrollWrites=0;
 const element={scrollHeight:1400,clientHeight:400,get scrollTop(){return top;},set scrollTop(value){scrollWrites++;top=Math.min(value,this.scrollHeight-this.clientHeight);}};
 const render=()=>React.createElement(ConversationFeed,{revision,key:generation},React.createElement('p',null,'synthetic dialogue'));
 await act(async()=>{tree=create(render(),{createNodeMock:el=>el.props.role==='region'?element:null});});
 const region=()=>tree.root.findByProps({role:'region'});
 const click=async label=>{const button=tree.root.findAllByType('button').find(b=>text(b)===label);assert.ok(button,label);await act(async()=>button.props.onClick());};
 const update=async()=>{element.scrollHeight+=40;revision++;await act(async()=>tree.update(render()));};
 assert.match(text(tree.toJSON().at(-1)),/阅读模式/);
 await update();assert.equal(top,100);assert.equal(scrollWrites,0);assert.match(text(tree.toJSON().at(-1)),/有新内容/);
 await click('回到最新对话');const oneShot=top;assert.equal(top,1040);
 await update();assert.equal(top,oneShot,'one-shot navigation enabled automatic following');
 await click('跟随最新对话');await update();assert.equal(top,element.scrollHeight-400);
 for(const interaction of [p=>p.onPointerDownCapture(),p=>p.onFocusCapture(),p=>p.onWheel({deltaY:-1}),p=>p.onTouchMove(),p=>p.onKeyDown({key:'PageUp'}),p=>{top-=80;p.onScroll();}]){
   await act(async()=>interaction(region().props));const held=top;await update();assert.equal(top,held);
   top=element.scrollHeight-400;await act(async()=>region().props.onScroll());await update();assert.equal(top,element.scrollHeight-440,'near-bottom scroll opted into following');
   await click('跟随最新对话');
 }
 const before=scrollWrites;await act(async()=>tree.update(render()));assert.equal(scrollWrites,before,'unrelated rerender moved the viewport');
 await click('停止跟随');await update();assert.match(text(tree.toJSON().at(-1)),/阅读模式/);
 await click('跟随最新对话');generation++;await act(async()=>tree.update(render()));
 assert.match(text(tree.toJSON().at(-1)),/阅读模式/);assert.ok(!text(tree.toJSON().at(-1)).includes('有新内容'));
 await act(async()=>tree.unmount());
 console.log('PASS: default reading mode, explicit follow/one-shot navigation, scroll/pointer/focus/wheel/touch/keyboard pause, no implicit re-enable, unrelated updates and reset');
})().catch(e=>{console.error(e);process.exitCode=1;});
