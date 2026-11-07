import content from '../../../education/content-packs.json';
import type {Turn, Source} from './interview-session';

export const packs=content.packs;
export const studyLimits={attempts:500,revisions:1000,tasks:1000} as const;
export type Provenance='actual_self_speech'|'typed_self_response'|'ai_draft'|'ai_interviewer'|'hint'|'reference_solution'|'external_source'|'synthetic_fixture';
export type Assistance='independent'|'prompted'|'worked_example'|'unknown';
export type HelpEvent={id:string;kind:'feedback'|'hint'|'reference_solution'|'unknown';ref:string;at:string};
export type Revision={revision:number;text:string;hash:string;input_source:Provenance;source_ref:string|null;assistance_level:Assistance;answer_exposed:boolean;help_events:HelpEvent[];created_at:string};
export type Attempt={id:string;question_id:string;question_version:number;question:string;revisions:Revision[];help_events:HelpEvent[];answer_exposed:boolean;assistance_level:Assistance;created_at:string};
export type Evidence={id:string;text:string;hash:string;provenance:Provenance;location:string;allow_external:boolean};
export type Ref={id:string;hash:string;quote:string;start:number;end:number};
export type Criterion={criterion_id:string;judgment:string;score:0|1|2|3|'N/A'|'evidence_insufficient';evidence_refs:Ref[];source_refs:Ref[];next_action:string};
export type Feedback={id:string;task_id:string;attempt_id:string;answer_revision:number;rubric_version:string;model_version:string;criteria:Criterion[];evidence_snapshot:Evidence[];demonstrated:string;priority_improvement:string;next_action:string;status:'current'|'stale';supersedes:string|null;independent_assessment:boolean};
export type Task={task_id:string;session_id:string;generation:number;input_hash:string;attempt_id:string;answer_revision:number;lane:'interactive'|'background';state:'queued'|'running'|'completed'|'partial'|'failed'|'cancelled';budget:number|null;cancel_requested:boolean;output_revision:number|null;error:string;usage:number|null;request_attempts:number;raw_output:string;reported:boolean};
export type Draft={question:string;question_ref:string|null;text:string;input_source:Provenance;assistance_level:Assistance;source_ref:string|null;source_draft?:string};
export type Study={schema_version:1;format:'tingxian-learning-v1';id:string;mode:'review';goal:string;pack_id:string;pack_version:string;source_session_ref:string|null;generation:number;status:'editing'|'saved'|'archived';consent_snapshot:null|{task_id:string;input_hash:string;profile_id:string};draft:Draft;attempts:Attempt[];feedback:Feedback[];tasks:Task[];evidence:Evidence[];items:{id:string;goal:string;attempt_refs:string[];next_action:string;review_due:string|null;scheduler_version:null;self_assessment:string}[];preferences:{profile_id:string;context_limit:number;font_size:16|20|24};store_revision:number};
export type Request={task_id:string;session_id:string;generation:number;lane:Task['lane'];input_json:string;input_hash:string;profile_id:string;context_limit:number;confirmed:boolean;profile_stamp?:string;scope_epoch?:number};
export type Result={text:string;model_version:string;usage:number|null;request_attempts:number;error:string|null;budget:number};
export const clone=<T,>(v:T):T=>JSON.parse(JSON.stringify(v));
const uid=()=>crypto.randomUUID();
const now=()=>new Date().toISOString();
const selfSources:Provenance[]=['actual_self_speech','typed_self_response','synthetic_fixture'];
export async function sha(text:string):Promise<string>{return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(text))),v=>v.toString(16).padStart(2,'0')).join('');}
export function newStudy():Study{return {schema_version:1,format:'tingxian-learning-v1',id:uid(),mode:'review',goal:packs[0].questions[0].goal,pack_id:packs[0].id,pack_version:packs[0].version,source_session_ref:null,generation:0,status:'editing',consent_snapshot:null,draft:{question:'',question_ref:null,text:'',input_source:'typed_self_response',assistance_level:'unknown',source_ref:null},attempts:[],feedback:[],tasks:[],evidence:[],items:[],preferences:{profile_id:'answer-1',context_limit:32768,font_size:20},store_revision:0};}
export function answerChoices(history:Turn[]){return history.flatMap(t=>t.role==='self'?[{id:t.id,text:t.question,provenance:'actual_self_speech' as Provenance}]:t.confirmed_answer?[{id:t.id,text:t.confirmed_answer,provenance:'typed_self_response' as Provenance}]:[]);}
export async function sourceEvidence(s:Source):Promise<Evidence>{return {id:s.source_id,text:s.text,hash:await sha(s.text),provenance:s.kind==='practice'?'reference_solution':'external_source',location:s.location,allow_external:s.allow_external};}
export function invalidate(study:Study):void{study.generation++;study.consent_snapshot=null;study.tasks.forEach(t=>{if(['queued','running'].includes(t.state)){t.state='cancelled';t.cancel_requested=true;}});}
export function editDraft(study:Study,change:Partial<Draft>):void{invalidate(study);study.draft={...study.draft,...change};study.status='editing';}
export async function commitAttempt(study:Study,reviseId?:string):Promise<void>{
  const draft=clone(study.draft),generation=study.generation;
  if(!draft.question.trim()||!draft.text.trim())throw Error('请填写题目和本人作答；AI 草稿不能补齐初答');
  if(!selfSources.includes(draft.input_source))throw Error('该来源不是本人作答');
  if(draft.question.length>12000||draft.text.length>50000)throw Error('单题或回答过长，请拆分后保存');
  const hash=await sha(draft.text);if(study.generation!==generation)throw Error('输入已变化，请重新保存');
  const old=reviseId?study.attempts.find(a=>a.id===reviseId):undefined;
  if(reviseId&&!old)throw Error('原尝试不存在');
  if(!old&&study.attempts.length>=studyLimits.attempts)throw Error('本份复盘作答数已达上限，请保存草稿后新建练习；当前输入保留');
  if(old&&old.revisions.length>=studyLimits.revisions)throw Error('本次作答修订数已达上限，请保存草稿后新建练习；当前输入保留');
  if(old&&old.question!==draft.question)throw Error('题目变化请新建尝试，不能覆盖旧题');
  // A text correction or another attempt at the same question cannot erase
  // help already recorded for that question. A different question can still
  // provide independent evidence; no old answer or feedback is rewritten.
  const related=study.attempts.filter(a=>a===old||a.question===draft.question||!!draft.question_ref&&a.question_id===draft.question_ref);
  const prior=related.flatMap(a=>a.revisions),feedback=study.feedback.filter(f=>related.some(a=>a.id===f.attempt_id));
  const help:HelpEvent[]=clone([...new Map(prior.flatMap(r=>r.help_events).map(h=>[h.id,h])).values()]);
  const record=(kind:HelpEvent['kind'],ref:string)=>{if(!help.some(h=>h.kind===kind&&h.ref===ref))help.push({id:uid(),kind,ref,at:now()});};
  for(const f of feedback)record('feedback',f.id);
  for(const a of related)for(const r of a.revisions){
    // v1 records may describe assistance without a corresponding help event.
    if(r.answer_exposed||r.assistance_level==='worked_example'){if(!r.help_events.some(h=>h.kind==='reference_solution'))record('reference_solution',`${a.id}:${r.revision}`);}
    else if(r.assistance_level==='prompted'){if(!r.help_events.some(h=>h.kind==='hint'||h.kind==='feedback'))record('hint',`${a.id}:${r.revision}`);}
    else if(r.assistance_level==='unknown'){if(!r.help_events.some(h=>h.kind==='unknown'))record('unknown',`${a.id}:${r.revision}`);}
  }
  if(study.source_session_ref)record('unknown',study.source_session_ref);
  for(const e of study.evidence)if(e.provenance==='reference_solution')record('reference_solution',e.id);
  if(draft.assistance_level!=='independent')record(draft.assistance_level==='worked_example'?'reference_solution':draft.assistance_level==='prompted'?'hint':'unknown','self-report');
  const exposed=help.some(h=>h.kind==='reference_solution');
  const assistance:Assistance=exposed?'worked_example':help.some(h=>h.kind==='feedback'||h.kind==='hint')?'prompted':help.some(h=>h.kind==='unknown')?'unknown':'independent';
  const revision:Revision={revision:(old?.revisions.length??0)+1,text:draft.text,hash,input_source:draft.input_source,source_ref:draft.source_ref,assistance_level:assistance,answer_exposed:exposed,help_events:help,created_at:now()};
  invalidate(study);
  if(old){old.revisions.push(revision);study.feedback.filter(f=>f.attempt_id===old.id).forEach(f=>f.status='stale');}
  else {
    study.attempts.push({id:uid(),question_id:draft.question_ref??uid(),question_version:1,question:draft.question,revisions:[revision],help_events:help,answer_exposed:exposed,assistance_level:assistance,created_at:now()});
  }
}
export async function prepareRequest(study:Study,attemptId:string,lane:Task['lane']):Promise<Request>{
  if(study.tasks.length>=studyLimits.tasks)throw Error('本份复盘任务数已达上限，请保存草稿后新建练习；当前输入保留');
  const generation=study.generation,attempt=study.attempts.find(a=>a.id===attemptId);if(!attempt)throw Error('先保存本人作答');
  const answer=attempt.revisions.at(-1)!;
  if(study.draft.text!==answer.text||study.draft.question!==attempt.question)throw Error('输入已修改，请先保存修订或新的尝试');
  const selected=study.evidence.filter(e=>e.allow_external);
  const input={schema:'tingxian-learning-input-v1',pack_id:study.pack_id,pack_version:study.pack_version,goal:study.goal,question:attempt.question,attempt_id:attempt.id,answer_revision:answer.revision,answer:answer.text,answer_hash:answer.hash,input_source:answer.input_source,assistance_level:answer.assistance_level,answer_exposed:answer.answer_exposed,evidence:selected};
  const input_json=JSON.stringify(input),input_hash=await sha(input_json);
  if(generation!==study.generation)throw Error('本次使用范围已变化，请重新预览');
  return {task_id:uid(),session_id:study.id,generation,lane,input_json,input_hash,profile_id:study.preferences.profile_id,context_limit:study.preferences.context_limit,confirmed:false};
}
const text=(v:unknown,limit=4000):v is string=>typeof v==='string'&&v.trim().length>0&&v.length<=limit;
function exactKeys(v:any,keys:string[]){if(!v||typeof v!=='object'||Array.isArray(v)||Object.keys(v).some(k=>!keys.includes(k))||keys.some(k=>!(k in v)))throw Error('反馈结构无效');}
function references(raw:any,sources:Evidence[]):Ref[]{
  if(!Array.isArray(raw)||raw.length>8)throw Error('反馈引用格式无效');
  return raw.map(r=>{exactKeys(r,['id','hash','quote']);const source=sources.find(s=>s.id===r.id&&s.hash===r.hash);
    if(!source||!text(r.quote,12000))throw Error('引用来源或摘要不存在');
    const start=source.text.indexOf(r.quote);if(start<0)throw Error('引用无法定位到所选原文');
    return {...r,start,end:start+r.quote.length};});
}
export function validateFeedback(raw:string,study:Study,task:Task,model:string):Feedback{
  let value:any;try{value=JSON.parse(raw);}catch{throw Error('模型未返回有效 JSON；输入和返回文本已保留');}
  exactKeys(value,['schema','demonstrated','priority_improvement','next_action','criteria']);
  if(value.schema!=='tingxian-feedback-v1'||![value.demonstrated,value.priority_improvement,value.next_action].every(v=>text(v)))throw Error('反馈为空或版本无效');
  const attempt=study.attempts.find(a=>a.id===task.attempt_id)!,answer=attempt?.revisions.find(r=>r.revision===task.answer_revision);
  if(!answer)throw Error('作答修订已不存在');
  const pack=packs.find(p=>p.id===study.pack_id)!;
  if(!Array.isArray(value.criteria)||value.criteria.length!==pack.criteria.length)throw Error('反馈维度不完整');
  const ids=new Set<string>();
  const criteria=value.criteria.map((c:any)=>{
    exactKeys(c,['criterion_id','judgment','score','evidence_refs','source_refs','next_action']);
    if(!pack.criteria.some(x=>x.id===c.criterion_id)||ids.has(c.criterion_id)||![c.judgment,c.next_action].every(v=>text(v))||![0,1,2,3,'N/A','evidence_insufficient'].includes(c.score))throw Error('评分或维度无效');ids.add(c.criterion_id);
    const evidence_refs=references(c.evidence_refs,[{id:attempt.id,text:answer.text,hash:answer.hash,provenance:answer.input_source,location:'本人作答',allow_external:true}]);
    const source_refs=references(c.source_refs,study.evidence.filter(e=>e.allow_external));
    if(typeof c.score==='number'&&!evidence_refs.length)throw Error('无本人作答证据，不能给出分数');
    return {...c,evidence_refs,source_refs};
  });
  return {id:task.task_id,task_id:task.task_id,attempt_id:attempt.id,answer_revision:answer.revision,rubric_version:pack.rubric_version,model_version:model,criteria,evidence_snapshot:clone(study.evidence.filter(e=>e.allow_external)),demonstrated:value.demonstrated,priority_improvement:value.priority_improvement,next_action:value.next_action,status:'current',supersedes:study.feedback.findLast(f=>f.attempt_id===attempt.id)?.id??null,independent_assessment:answer.assistance_level==='independent'&&!answer.answer_exposed&&answer.input_source!=='synthetic_fixture'};
}
export function applyResult(study:Study,request:Request,result:Result):boolean{
  const task=study.tasks.find(t=>t.task_id===request.task_id);
  if(!task||task.reported||task.input_hash!==request.input_hash)return false;
  task.usage=result.usage;task.request_attempts=result.request_attempts;task.budget=result.budget;
  task.reported=true;
  if(task.state!=='running'||task.generation!==study.generation)return false;
  task.raw_output=result.text;
  if(result.error){task.state=result.text?'partial':'failed';task.error=result.error;return true;}
  try{
    const feedback=validateFeedback(result.text,study,task,result.model_version);
    if(!study.feedback.some(f=>f.task_id===task.task_id)){study.feedback.filter(f=>f.attempt_id===feedback.attempt_id).forEach(f=>f.status='stale');study.feedback.push(feedback);study.items.push({id:task.task_id,goal:study.goal,attempt_refs:[feedback.attempt_id],next_action:feedback.next_action,review_due:null,scheduler_version:null,self_assessment:''});}
    task.state='completed';task.output_revision=feedback.answer_revision;
  }catch(e){task.state='failed';task.error=String(e);}
  return true;
}
export async function restoreStudy(raw:unknown):Promise<Study>{
  const s=clone(raw) as Study;
  if(!s||s.format!=='tingxian-learning-v1'||s.schema_version!==1||s.mode!=='review'||!text(s.id,100)||!packs.some(p=>p.id===s.pack_id&&p.version===s.pack_version)||!Number.isSafeInteger(s.generation)||s.generation<0||!Number.isSafeInteger(s.store_revision)||s.store_revision<0)throw Error('教育记录版本或结构无效，保留原文件');
  if(!s.draft||typeof s.draft.question!=='string'||typeof s.draft.text!=='string'||!Array.isArray(s.attempts)||!Array.isArray(s.feedback)||!Array.isArray(s.tasks)||!Array.isArray(s.evidence)||!Array.isArray(s.items)||s.attempts.length>studyLimits.attempts||s.tasks.length>studyLimits.tasks)throw Error('教育记录内容无效，保留原文件');
  if(!s.preferences||![16,20,24].includes(s.preferences.font_size)||!Number.isSafeInteger(s.preferences.context_limit)||s.preferences.context_limit<8192||s.preferences.context_limit>262144||!['independent','prompted','worked_example','unknown'].includes(s.draft.assistance_level)||!selfSources.includes(s.draft.input_source))throw Error('教育偏好或输入来源无效');
  const unique=(a:{id:string}[])=>{const ids=new Set<string>();for(const item of a){if(!text(item.id,128)||ids.has(item.id))throw Error('教育条目标识重复或无效');ids.add(item.id);}};
  unique(s.attempts);unique(s.feedback);unique(s.evidence);unique(s.items);unique(s.tasks.map(t=>({id:t.task_id})));
  for(const a of s.attempts){if(!text(a.question,12000)||!Array.isArray(a.revisions)||!a.revisions.length||!['independent','prompted','worked_example','unknown'].includes(a.assistance_level)||!Array.isArray(a.help_events)||typeof a.answer_exposed!=='boolean')throw Error('作答记录无效');
    for(const [i,r] of a.revisions.entries()){if(r.revision!==i+1||!text(r.text,50000)||await sha(r.text)!==r.hash||!selfSources.includes(r.input_source)||!['independent','prompted','worked_example','unknown'].includes(r.assistance_level)||!Array.isArray(r.help_events)||typeof r.answer_exposed!=='boolean')throw Error('作答修订无效');}}
  for(const e of s.evidence){if(!text(e.text,64000)||await sha(e.text)!==e.hash||!['external_source','reference_solution','synthetic_fixture'].includes(e.provenance)||typeof e.allow_external!=='boolean')throw Error('资料摘要或来源无效');}
  for(const t of s.tasks){if(!['queued','running','completed','partial','failed','cancelled'].includes(t.state)||t.session_id!==s.id||!Number.isSafeInteger(t.generation)||t.generation>s.generation||!(t.usage===null||(Number.isSafeInteger(t.usage)&&t.usage>=0))||!Number.isInteger(t.request_attempts)||t.request_attempts<0||t.request_attempts>2||!s.attempts.some(a=>a.id===t.attempt_id&&a.revisions.some(r=>r.revision===t.answer_revision)))throw Error('评估任务记录无效');}
  for(const f of s.feedback){
    const task=s.tasks.find(t=>t.task_id===f.task_id);if(!task||task.state!=='completed'||!['current','stale'].includes(f.status)||f.answer_revision!==task.answer_revision||f.attempt_id!==task.attempt_id)throw Error('反馈关联记录无效');
    // Revalidate saved citations as well as transport JSON. Stored offsets may
    // not be used to attach changed text to an earlier score.
    const raw={schema:'tingxian-feedback-v1',demonstrated:f.demonstrated,priority_improvement:f.priority_improvement,next_action:f.next_action,criteria:f.criteria.map(c=>({...c,evidence_refs:c.evidence_refs.map(({id,hash,quote})=>({id,hash,quote})),source_refs:c.source_refs.map(({id,hash,quote})=>({id,hash,quote}))}))};
    if(!Array.isArray(f.evidence_snapshot))throw Error('缺少反馈证据快照');for(const e of f.evidence_snapshot){if(await sha(e.text)!==e.hash)throw Error('旧反馈证据摘要无效');}
    const checked=validateFeedback(JSON.stringify(raw),{...s,evidence:f.evidence_snapshot},task,f.model_version);
    // Native serde_json and browser JSON may order object keys differently.
    const canonical=(v:any):string=>JSON.stringify(v,(_k,x)=>x&&typeof x==='object'&&!Array.isArray(x)?Object.fromEntries(Object.keys(x).sort().map(k=>[k,x[k]])):x);
    if(canonical(checked.criteria)!==canonical(f.criteria)||checked.rubric_version!==f.rubric_version||checked.independent_assessment!==f.independent_assessment)throw Error('反馈引用或版本校验失败');
  }
  invalidate(s);return s;
}
