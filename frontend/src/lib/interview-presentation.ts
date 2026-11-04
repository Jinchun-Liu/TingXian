import type {Turn} from './interview-session';

export const QUESTION_MARKER = '<<<QUESTION>>>';
export type FontSize = 'small' | 'medium' | 'large';
export type PresentationPreferences = {fontSize:FontSize;showSelf:boolean;focus:boolean};
export const presentationDefaults:PresentationPreferences = {fontSize:'medium',showSelf:false,focus:true};
export const fontPixels:Record<FontSize,number> = {small:16,medium:20,large:24};
export type DisplayTurn = Pick<Turn,'id'|'role'|'question'|'suggestion'|'status'|'time'>;

/** Keep the protocol tail out of the streaming answer, including when the
 * delimiter or heading arrives one character at a time. Older saved answers
 * remain readable. A missing correction never invents a corrected question. */
export function answerPresentation(raw:string, completed:boolean, original:string) {
  let text=raw.replace(/\r\n/g,'\n');
  const marker=text.indexOf(QUESTION_MARKER);
  const corrected=marker>=0?text.slice(marker+QUESTION_MARKER.length).trim():'';
  if(marker>=0)text=text.slice(0,marker);
  else {
    for(let n=Math.min(text.length,QUESTION_MARKER.length-1);n>0;n--){
      if(text.endsWith(QUESTION_MARKER.slice(0,n))){text=text.slice(0,-n);break;}
    }
  }
  const trimmed=text.trimStart();
  if(!completed && '建议回答：'.startsWith(trimmed))text='';
  else text=text.replace(/^\s*(?:#{1,6}\s*)?(?:\*\*)?建议回答(?:\*\*)?\s*[：:]\s*/u,'');
  text=text.split(/\n\s*听写纠正\s*[：:]/u)[0].trimEnd();
  return {answer:text,question:completed?(corrected||original):'',corrected:completed&&!!corrected};
}

export function visibleTurns(history:DisplayTurn[], showSelf:boolean) {
  return showSelf?history:history.filter(t=>t.role!=='self');
}
export function readPresentationPreferences(value:unknown):PresentationPreferences {
  const v=value as Partial<PresentationPreferences>|null;
  return {fontSize:v && ['small','medium','large'].includes(v.fontSize||'')?v.fontSize!:presentationDefaults.fontSize,
    showSelf:v?.showSelf===true,focus:v?.focus!==false};
}

/** Each question owns its revision and stream buffer. Completing/cancelling a
 * different question cannot steal this slot or overwrite its text. */
export class AnswerRequests {
  private revision=0;
  private jobs=new Map<string,{revision:number;text:string;kind:'text'|'vision'}>();
  get size(){return this.jobs.size;}
  count(kind:'text'|'vision'){return this.entries().filter(job=>job.kind===kind).length;}
  begin(id:string,capacity:number,kind:'text'|'vision'='text'){
    if(this.jobs.has(id)||this.jobs.size>=3||this.count(kind)>=capacity)return null;
    const revision=++this.revision;this.jobs.set(id,{revision,text:'',kind});return revision;
  }
  get(id:string,revision:number){const job=this.jobs.get(id);return job?.revision===revision?job:undefined;}
  append(id:string,revision:number,delta:string){const job=this.get(id,revision);if(!job)return null;job.text+=delta;return job.text;}
  finish(id:string,revision:number){if(!this.get(id,revision))return false;return this.jobs.delete(id);}
  entries(){return [...this.jobs.entries()].map(([id,job])=>({id,...job}));}
  clear(){const jobs=this.entries();this.jobs.clear();return jobs;}
}
