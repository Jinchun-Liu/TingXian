// Standalone real-DOM fixture for the production reading component. No network,
// native bridge, profile data or microphone; refuses to overwrite an old run.
const fs=require('node:fs'),path=require('node:path');
const ts=require('../frontend/node_modules/typescript');
const output=process.argv[2];if(!output)throw Error('Pass a new output directory');
fs.mkdirSync(output,{recursive:false});
const runtime=name=>fs.readFileSync(path.resolve(__dirname,'../frontend/node_modules',name),'utf8');
const component=ts.transpileModule(fs.readFileSync(path.resolve(__dirname,'../frontend/src/components/interview/ConversationFeed.tsx'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020,jsx:ts.JsxEmit.ReactJSX}}).outputText;
const html=`<!doctype html><meta charset="utf-8"><title>面试对话 · 合成阅读验证</title>
<style>body{font:16px system-ui;background:#f1f5f9;color:#1e293b;margin:24px}main{max-width:960px;margin:auto}button{font:inherit;font-size:13px;background:white;border:1px solid #cbd5e1;border-radius:6px;padding:7px 12px;cursor:pointer}button[aria-pressed=true]{background:#0f172a;color:white}input{padding:8px;margin-top:16px;width:90%}.controls,.flex{display:flex;gap:8px;align-items:center;flex-wrap:wrap}.controls{margin:16px 0}.justify-between{justify-content:space-between}.border-t{border-top:1px solid #cbd5e1}.px-4{padding-left:16px;padding-right:16px}.py-2{padding-top:8px;padding-bottom:8px}.text-xs{font-size:12px}.text-slate-500{color:#64748b}[role=region]{height:400px;overflow-y:auto;padding:20px;background:#f8fafc;border:1px solid #cbd5e1}article{background:white;margin-bottom:22px;border:1px solid #e2e8f0;border-radius:8px;padding:16px}p{line-height:1.65}summary{cursor:pointer;color:#475569}h2{font-size:17px}h1{font-size:24px}</style>
<main><h1>面试对话阅读验证</h1><p>仅合成数据；不调用 AI、不采集音频。使用生产 ConversationFeed 组件。</p><div id="root"></div></main>
<script>${runtime('react/umd/react.development.js')}</script><script>${runtime('react-dom/umd/react-dom.development.js')}</script>
<script>(()=>{const m={exports:{}};const jsx=(type,props,key)=>React.createElement(type,{...props,key});const jsxs=(type,{children,...props},key)=>React.createElement(type,{...props,key},...children);const require=name=>name==='react'?React:name==='react/jsx-runtime'?{jsx,jsxs,Fragment:React.Fragment}:{Button:({variant,size,...props})=>React.createElement('button',props)};(function(require,module,exports){${component}\n})(require,m,m.exports);window.ConversationFeed=m.exports.ConversationFeed;})();</script>
<script>
function Fixture(){const [rows,setRows]=React.useState(()=>initial());const [generation,setGeneration]=React.useState(0);const [streaming,setStreaming]=React.useState(false);
function initial(){return Array.from({length:20},(_,i)=>({id:i+1,text:'我会分别验证 JSON Schema 与业务语义，覆盖 null、零、空字符串和缺失字段。'}));}
const append=()=>setRows(old=>old.length?old.map((row,i)=>i===old.length-1?{...row,text:row.text+' 新增流式建议：幂等键还需要结合租户隔离与重试窗口验证。'}:row):old);
React.useEffect(()=>{if(!streaming)return;const timer=setInterval(append,150);return()=>clearInterval(timer);},[streaming]);
const button=(id,title,action)=>React.createElement('button',{id,onClick:action},title);
return React.createElement(React.Fragment,null,
 React.createElement('div',{className:'controls'},button('add','新增对方问题',()=>setRows(old=>[...old,{id:(old.at(-1)?.id||0)+1,text:'接口重试怎样避免重复写入？'}])),button('append','流式追加一句',append),button('stream',streaming?'停止模拟输出':'开始模拟输出',()=>setStreaming(!streaming)),button('clear','清空演示记录',()=>{setRows([]);setGeneration(v=>v+1);}),button('restore','恢复合成历史',()=>{setRows(initial());setGeneration(v=>v+1);})),
 React.createElement(ConversationFeed,{revision:rows,key:generation},rows.map(row=>React.createElement('article',{key:row.id,id:'row-'+row.id},React.createElement('h2',null,'第 '+row.id+' 题 · 契约测试'),React.createElement('p',null,row.text),React.createElement('details',null,React.createElement('summary',null,'参考来源 '+row.id),React.createElement('p',null,'合成来源：DemoProject、FastAPI、pytest、Doris FE/BE、WAL、MVCC。这里的示例不代表个人经历。'))))),
 React.createElement('input',{id:'composer',placeholder:'模拟手动输入：输出更新不应抢走焦点'}));}
ReactDOM.createRoot(document.getElementById('root')).render(React.createElement(Fixture));
</script>`;
fs.writeFileSync(path.join(output,'reading.html'),html);console.log(path.join(output,'reading.html'));
