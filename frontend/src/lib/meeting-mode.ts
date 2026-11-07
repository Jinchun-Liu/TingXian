export const meetingModes = {
  pair: '一对一', panel: '多对一', group: '一对多', groups: '多对多', mixed: '复合会议',
} as const;
export type MeetingMode = keyof typeof meetingModes;
export type MeetingSetup = {mode: MeetingMode; names: string};
export const initialMeeting: MeetingSetup = {mode:'pair', names:''};
export const addressedOnly = (mode: MeetingMode) => ['group','groups','mixed'].includes(mode);
export function readMeeting(value: unknown): MeetingSetup {
  const config = value as Partial<MeetingSetup> | null;
  return {mode: config?.mode && Object.prototype.hasOwnProperty.call(meetingModes,config.mode) ? config.mode : 'pair',
    names: typeof config?.names === 'string' ? config.names.slice(0,240) : ''};
}
const normalize = (s:string) => s.normalize('NFKC').toLowerCase().trim();
const aliases = (s:string) => [...new Set(s.split(/[,，、;；\n]/).map(normalize).filter(s=>s.length>=1 && s.length<=32))].sort((a,b)=>b.length-a.length);
const request = /^(?:(?:现在|now\b)[\s,，]*)?(?:please\b|over to you\b|这个问题你|这个问题您|你|您|请|来(?:说|讲|介绍)|能否|能不能|能说|怎么看|怎么样|说说|讲讲|谈谈|介绍|解释|分享|回答|轮到你|该你|can you\b|could you\b|would you\b|what\b|how\b|why\b)/iu;
const breaks = /^[\s,:：，、。.!！?？—-]+/u;
const literal = (s:string) => s.replace(/[.*+?^$(){}|[\]\\]/g,'\\$&');
export type MeetingDecision = {answer:boolean; alert:string; reminder?:'later'|'collective'|'cancelled'};
// Only a leading scheduling instruction counts. Words such as “稍后” inside
// a technical question must not turn an immediate question into a reminder.
const later = /^(?:(?:你|您|请|麻烦|先)\s*)*(?:稍后|待会儿?|一会儿?|等会儿?|等(?:大家|其他人|前面几位|讨论|这一轮)[^。！？!?；;]{0,24}(?:完|结束|之后|以后).*?再)/u;
const laterEnglish = /^(?:please\s+)?(?:(?:after\s+(?:everyone|the others|this discussion)\b)|(?:(?:you can\s+)?(?:answer|speak|summari[sz]e(?: (?:this|the discussion))?|share your (?:thoughts|answer))\s+(?:later|at the end|after everyone)\b))/iu;
const collective = /^(?:(?:那|接下来|现在|请|麻烦)[\s,，:：]*)*(?:(?:大家|每个人|各位|我们)(?:都)?(?:各(?:自)?|依次|轮流)(?:来)?(?:说|讲|分享|介绍|发言)|(?:everyone|each of you)[,\s]+(?:please\s+)?(?:take turns|share|say a few words)\b)/iu;
const isLater = (text:string) => later.test(text) || laterEnglish.test(text);
const decline = /^(?:(?:你|您|请|先)\s*)*(?:不用|不必|不要|别)(?:急着)?(?:回答|发言|说)|^(?:not you|no need to answer)\b/iu;
export function meetingNotice(decision:MeetingDecision): {text:string; persistent:boolean} | null {
  if(decision.reminder==='cancelled')return {text:'',persistent:false};
  if(decision.reminder==='collective')return {text:'等待轮次 · 大家依次发言',persistent:false};
  if(!decision.alert)return null;
  return {text:(decision.reminder==='later'?'稍后发言 · ':'正在叫你 · ')+decision.alert,persistent:decision.reminder==='later'};
}
export class MeetingAddress {
  private until = 0;
  private pendingName = '';
  reset() { this.until=0; this.pendingName=''; }
  decide(setup:MeetingSetup, raw:string, now=Date.now()): MeetingDecision {
    if(!addressedOnly(setup.mode)){this.reset();return {answer:true,alert:''};}
    const pendingName=now<this.until?this.pendingName:''; this.reset();
    const text=normalize(raw).replace(/[“「『"][^”」』"]*[”」』"]/gu,' 引文 ');
    for(const name of aliases(setup.names)) {
      const pattern=(/^[a-z0-9]/i.test(name)?'(?<![a-z0-9_])':'')+literal(name)+(/[a-z0-9]$/i.test(name)?'(?![a-z0-9_])':'');
      for(const m of text.matchAll(new RegExp(pattern,'gu'))) {
        const start=m.index??0, before=text.slice(0,start).trim(), after=text.slice(start+m[0].length);
        const tail=after.replace(/^(?:同学|老师|先生|女士)(?=[\s,，:：]|你|您|请|$)/u,'').replace(breaks,'').trim();
        const clause=before.split(/[。？！?!；;\n]/u).at(-1)?.trim()??'';
        // Require a vocative or an explicit hand-off; a third-person mention is common in groups.
        const called=/^(?:(?:那[么就]?|好的?|接下来|下面|现在|然后|我们|想|先|一个问题|请问|我想问问|问一下|请|有请|让|麻烦|轮到|交给|我想请|再请|稍后|待会儿?|一会儿?|等会儿?|最后|hi|hey|okay|ok|over to)[\s,，:：]*)*$/iu.test(clause);
        const trailing=(!tail && /[?？,，]\s*$/u.test(before) && /(?:你|您|请|怎么看|能否|can you|what|how)/iu.test(before));
        if(!called&&!trailing)continue;
        if(/^(?:的|说过|提过|刚才说|那份|表示|认为|告诉我|said\b|told\b|['’]s\b)/iu.test(tail))continue;
        if(decline.test(tail))return {answer:false,alert:'',reminder:'cancelled'};
        if(isLater(tail) || isLater(clause))return {answer:false,alert:name,reminder:'later'};
        const handoff=/^(?:轮到你了|到你了|准备一下|下一位|your turn|over to you)[.!！。\s]*$/iu.test(tail);
        if((!tail && !trailing)||handoff){this.until=now+12000;this.pendingName=name;return {answer:false,alert:name};}
        if(trailing || request.test(tail) || (breaks.test(after) && /[?？]|为什么|如何|怎么|什么|是否|能否|呢[。.]?$/u.test(tail))){
          return {answer:true,alert:name};
        }
      }
    }
    const pendingText=text.replace(breaks,'');
    if(pendingName && decline.test(pendingText))return {answer:false,alert:'',reminder:'cancelled'};
    if(pendingName && isLater(pendingText))return {answer:false,alert:pendingName,reminder:'later'};
    if(aliases(setup.names).length>0 && collective.test(text))return {answer:false,alert:'',reminder:'collective'};
    return {answer:!!pendingName && aliases(setup.names).length>0 && request.test(text.replace(breaks,'')),alert:''};
  }
}
