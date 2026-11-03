// Build a standalone, isolated browser fixture from the actual recovery service.
// Run with a new output directory, serve it on localhost, click Run checks.
const fs = require('node:fs');
const path = require('node:path');
const ts = require('../frontend/node_modules/typescript');
const destination = process.argv[2];
if (!destination) throw new Error('output directory required');
fs.mkdirSync(destination, { recursive: true });
const input = fs.readFileSync(path.join(__dirname, '../frontend/src/services/indexedDBService.ts'), 'utf8');
const compiled = ts.transpileModule(input.replace("'MeetilyRecoveryDB'", "'MeetilyRecoveryDB-CleanupTest-' + Date.now()"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 }
}).outputText;
fs.writeFileSync(path.join(destination, 'service.js'), 'var exports = {};\n' + compiled);
fs.writeFileSync(path.join(destination, 'index.html'), `<!doctype html><html lang="zh"><meta charset="utf-8"><title>History recovery isolated checks</title>
<style>body{font:16px system-ui;margin:40px;max-width:900px}pre{white-space:pre-wrap}button{padding:12px}</style>
<h1>历史清理：隔离恢复缓存检查</h1><p>使用独立测试数据库，不读取真实应用缓存。</p><button id="run">运行检查</button><pre id="result">尚未运行</pre>
<script src="service.js"></script><script>
const output = document.getElementById('result');
const verify = (condition, message) => { if (!condition) throw new Error(message); };
const service = exports.indexedDBService;
document.getElementById('run').onclick = async () => {
 document.getElementById('run').disabled = true;
 const messages = [];
 const report = text => { messages.push(text); output.textContent = messages.join('\\n'); };
 try {
  await service.init();
  const metadata = (meetingId, title) => ({meetingId,title,startTime:1,lastUpdated:1,transcriptCount:0,savedToSQLite:false});
  await service.saveMeetingMetadata(metadata('owned','Interview practice'));
  await service.saveMeetingMetadata(metadata('other','Unrelated meeting'));
  await service.saveTranscript('owned',{text:'synthetic old answer',timestamp:'00:01',sequenceId:1,confidence:1});
  await service.saveTranscript('other',{text:'unrelated history',timestamp:'00:01',sequenceId:2,confidence:1});
  await service.markMeetingSaved('owned');
  const originalGetAll=IDBObjectStore.prototype.getAll;
  IDBObjectStore.prototype.getAll=function(...args){if(this.name==='transcripts')throw Error('startup inspection read transcript bodies');return originalGetAll.apply(this,args);};
  let inspected;
  try{inspected=await service.inspectInterviewHistory();}finally{IDBObjectStore.prototype.getAll=originalGetAll;}
  verify(inspected.meetings===1 && inspected.transcripts===1,'metadata inspection missed owned or included unrelated records');
  verify(!JSON.stringify(inspected).includes('synthetic old answer'),'inspection returned speech text');
  report('PASS: residue inspection uses metadata and index counts only, including saved interview cache');
  const initial = await service.snapshotInterviewHistory();
  verify(initial.meetings.length===1 && initial.transcripts.length===1 && initial.meetings[0].savedToSQLite,'snapshot must also include saved interview recovery');
  report('PASS: snapshot selects only interview recovery records');
  await service.saveTranscript('owned',{text:'arrived after archive',timestamp:'00:02',sequenceId:3,confidence:1});
  let rejected=false;
  try { await service.removeArchivedInterviewHistory(initial); } catch { rejected=true; }
  verify(rejected,'changed snapshot was deleted');
  verify((await service.snapshotInterviewHistory()).transcripts.length===2,'failed clear lost new record');
  report('PASS: concurrent unarchived changes abort atomically; all old and new records remain');
  service.suspendRecordingWrites();
  await service.saveTranscript('owned',{text:'late event must not persist',timestamp:'00:03',sequenceId:4,confidence:1});
  const archive = await service.snapshotInterviewHistory();
  verify(archive.transcripts.length===2,'suspended write resurrected history');
  verify(archive.transcripts.some(t=>t.text==='synthetic old answer'),'archive missing old answer');
  await service.removeArchivedInterviewHistory(archive);
  const cleared = await service.snapshotInterviewHistory();
  verify(cleared.meetings.length===0 && cleared.transcripts.length===0,'clear left selected records');
  verify((await service.getTranscripts('other')).length===1,'unrelated recovery removed');
  report('PASS: archived interview cache clears on transaction commit; unrelated history stays');
  await service.saveTranscript('owned',{text:'late event after clear',timestamp:'00:04',sequenceId:5,confidence:1});
  verify((await service.getTranscripts('owned')).length===0,'late event resurrected deleted cache');
  service.resumeRecordingWrites();
  await service.saveMeetingMetadata(metadata('new','Interview practice'));
  await service.saveTranscript('new',{text:'fresh recording',timestamp:'00:01',sequenceId:6,confidence:1});
  const fresh = await service.snapshotInterviewHistory();
  verify(fresh.transcripts.length===1 && fresh.transcripts[0].text==='fresh recording','new capture contains old context');
  await service.removeArchivedInterviewHistory(fresh);
  report('PASS: late events are blocked; next capture starts with fresh content');
  report('ALL CHECKS PASSED');
 } catch (error) { report('FAIL: '+String(error)); }
};
</script></html>`);
console.log('Prepared isolated browser recovery fixture at ' + destination);
