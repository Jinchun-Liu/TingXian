'use client';
import {ChoiceMenu,ChoiceOption} from '@/components/ui/choice-menu';
import {Button} from '@/components/ui/button';
export type CapabilitySupport='unknown'|'supported'|'unsupported';
export type ModelCapabilities={text:CapabilitySupport;image:CapabilitySupport;streaming:CapabilitySupport;usage:CapabilitySupport;checked_on:string;reference:string;model:string;endpoint:string;context_tokens:number;max_output_tokens:number};
type Budget={context_tokens:number;max_output_tokens:number};
const today=()=>{const d=new Date();return `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,'0')}-${String(d.getDate()).padStart(2,'0')}`;};
export const matchingCapabilities=(value:ModelCapabilities|undefined,model:string,budget:Budget,endpoint:string)=>value&&value.model===model&&value.endpoint===endpoint&&value.context_tokens===budget.context_tokens&&value.max_output_tokens===budget.max_output_tokens?value:undefined;
export function capabilitiesError(value:ModelCapabilities|undefined,model:string,budget:Budget,endpoint:string){
 if(!value)return '';
 if(!matchingCapabilities(value,model,budget,endpoint))return '模型、地址或窗口已变化，请重新核对能力。';
 const date=new Date(value.checked_on+'T00:00:00Z');
 if(!/^\d{4}-\d{2}-\d{2}$/.test(value.checked_on)||!Number.isFinite(date.getTime())||date.toISOString().slice(0,10)!==value.checked_on||value.checked_on>today())return '请填写有效的能力核对日期，不能晚于今天。';
 if(!value.reference.trim()||value.reference.length>500)return '请填写能力核对来源。';
 return '';
}
export function CapabilityFields({value,model,endpoint,budget,prefix,disabled,onChange}:{value?:ModelCapabilities;model:string;endpoint:string;budget:Budget;prefix:string;disabled:boolean;onChange:(value:ModelCapabilities|undefined)=>void}){
 const draft:ModelCapabilities=value||{model,endpoint,context_tokens:budget.context_tokens,max_output_tokens:budget.max_output_tokens,text:'unknown',image:'unknown',streaming:'unknown',usage:'unknown',checked_on:'',reference:''};
 return <details><summary>模型能力 · {value?.checked_on?`人工核对 ${value.checked_on}`:'未核对'}</summary>
  <div className="grid sm:grid-cols-2 gap-2 mt-2">{([['text','文字输入'],['image','图片输入'],['streaming','流式响应'],['usage','返回用量']] as const).map(([key,label])=><label key={key}>{label}<ChoiceMenu aria-label={`${prefix} ${label}`} disabled={disabled} value={draft[key]} onValueChange={v=>onChange({...draft,[key]:v as CapabilitySupport})}><ChoiceOption value="unknown">未知</ChoiceOption><ChoiceOption value="supported">支持</ChoiceOption><ChoiceOption value="unsupported">不支持</ChoiceOption></ChoiceMenu></label>)}
   <label>核对日期<input aria-label={`${prefix} 能力核对日期`} type="date" className="block w-full rounded border p-2" max={today()} disabled={disabled} value={draft.checked_on} onChange={e=>onChange({...draft,checked_on:e.target.value})}/></label>
   <label>来源<input aria-label={`${prefix} 能力核对来源`} className="block w-full rounded border p-2" placeholder="服务商文档地址或核对说明" maxLength={500} disabled={disabled} value={draft.reference} onChange={e=>onChange({...draft,reference:e.target.value})}/></label>
  </div><p className="mt-2 text-xs text-slate-500">按服务商说明核对窗口、输出和能力。固定文字测试不能证明窗口或图片能力；更换模型、地址或窗口后需重核。</p>
  <Button size="sm" variant="outline" className="mt-2" disabled={disabled||!value} onClick={()=>onChange(undefined)}>清除能力记录</Button>
 </details>;
}
