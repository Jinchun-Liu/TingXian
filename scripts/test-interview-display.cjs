// Exercise the actual secondary webview: live events must win over a delayed
// initial IPC read, especially after the owner remounts with a different epoch.
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const req=require('node:module').createRequire(path.resolve(__dirname,'../frontend/package.json'));
const ts=req('typescript'),React=req('react'),{create,act}=req('react-test-renderer');
const calls=[];let listener,resolveInitial,removed=0;
const deferred=()=>new Promise(resolve=>{resolveInitial=resolve;});
let initial=deferred();
const code=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/app/interview-display/page.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.ReactJSX}}).outputText;
const m={exports:{}};
new Function('require','module','exports',code)(name=>{
  if(name==='@tauri-apps/api/core')return {invoke:async(command,args)=>{calls.push({command,args});if(command==='interview_presentation_get')return initial;if(command==='interview_presentation_action')return;throw Error('unexpected command '+command);}};
  if(name==='@tauri-apps/api/event')return {listen:async(name,fn)=>{assert.equal(name,'interview-presentation');listener=fn;return()=>removed++;}};
  if(name==='@/components/interview/AnswerConversation')return {AnswerConversation:props=>React.createElement('conversation',props)};
  return req(name);
},m,m.exports);
const snapshot=(epoch,version,question)=>({epoch,version,history:[{id:'q',role:'remote',question,suggestion:'建议回答：答案',status:'done',time:'00:01'}],preferences:{fontSize:'medium',focus:true,showSelf:false},selected_id:null});
(async()=>{
  let tree;await act(async()=>{tree=create(React.createElement(m.exports.default));});
  const current=()=>tree.root.findByType('conversation').props;
  await act(async()=>listener({payload:snapshot('new-owner',2,'新会话问题')}));
  assert.equal(current().history[0].question,'新会话问题');
  await act(async()=>resolveInitial(snapshot('old-owner',90,'旧会话问题')));
  assert.equal(current().history[0].question,'新会话问题','delayed old-epoch IPC snapshot replaced a newer live event');
  await act(async()=>listener({payload:snapshot('new-owner',1,'迟到旧版本')}));
  assert.equal(current().history[0].question,'新会话问题');
  await act(async()=>listener({payload:snapshot('new-owner',3,'当前完整问题')}));
  assert.equal(current().history[0].question,'当前完整问题');
  await act(async()=>current().onSelect('q'));assert.equal(current().selectedId,'q');assert.equal(current().onEdit,undefined);
  assert.deepEqual(calls.map(c=>c.command),['interview_presentation_get']);
  assert.ok(calls.every(c=>['interview_presentation_get','interview_presentation_action'].includes(c.command)),'display invoked capture/model work');
  await act(async()=>tree.unmount());assert.equal(removed,1);
  initial=deferred();await act(async()=>{tree=create(React.createElement(m.exports.default));});
  await act(async()=>resolveInitial(snapshot('current-owner',1,'启动快照')));
  assert.equal(current().history[0].question,'启动快照');
  await act(async()=>tree.unmount());assert.equal(removed,2);
  console.log('PASS: display live event beats stale initial read across owner epochs, older versions ignored, startup snapshot fallback, read-only actions and listener cleanup');
})().catch(e=>{console.error(e);process.exitCode=1;});
