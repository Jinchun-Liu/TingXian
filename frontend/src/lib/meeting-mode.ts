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
const request = /^(?:please\b|over to you\b|这个问题你|这个问题您|你|您|请|来(?:说|讲|介绍)|能否|能不能|能说|怎么看|怎么样|说说|讲讲|谈谈|介绍|解释|分享|回答|轮到你|该你|can you\b|could you\b|would you\b|what\b|how\b|why\b)/iu;
const breaks = /^[\s,:：，、。.!！?？—-]+/u;
const literal = (s:string) => s.replace(/[.*+?^$(){}|[\]\\]/g,'\\$&');
export class MeetingAddress {
  private until = 0;
  reset() { this.until=0; }
  decide(setup:MeetingSetup, raw:string, now=Date.now()): {answer:boolean; alert:string} {
    if(!addressedOnly(setup.mode)){this.reset();return {answer:true,alert:''};}
    const pending=now<this.until; this.reset();
    const text=normalize(raw).replace(/[“「『"][^”」』"]*[”」』"]/gu,' 引文 ');
    for(const name of aliases(setup.names)) {
      const pattern=(/^[a-z0-9]/i.test(name)?'(?<![a-z0-9_])':'')+literal(name)+(/[a-z0-9]$/i.test(name)?'(?![a-z0-9_])':'');
      for(const m of text.matchAll(new RegExp(pattern,'gu'))) {
        const start=m.index??0, before=text.slice(0,start).trim(), after=text.slice(start+m[0].length);
        const tail=after.replace(/^(?:同学|老师|先生|女士)(?=[\s,，:：]|你|您|请|$)/u,'').replace(breaks,'').trim();
        const clause=before.split(/[。？！?!；;\n]/u).at(-1)?.trim()??'';
        // Require a vocative or an explicit hand-off; a third-person mention is common in groups.
        const called=/^(?:(?:那[么就]?|好的?|接下来|下面|现在|然后|我们|想|先|一个问题|请问|我想问问|问一下|请|有请|让|麻烦|轮到|交给|我想请|再请|hi|hey|okay|ok|over to)[\s,，:：]*)*$/iu.test(clause);
        const trailing=(!tail && /[?？,，]\s*$/u.test(before) && /(?:你|您|请|怎么看|能否|can you|what|how)/iu.test(before));
        if(!called&&!trailing)continue;
        if(/^(?:的|说过|提过|刚才说|那份|表示|认为|告诉我|said\b|told\b|['’]s\b)/iu.test(tail))continue;
        if(/^(?:你|您)?(?:先)?(?:不用|不必|不要|别)(?:急着)?(?:回答|发言|说)|^(?:not you|no need to answer)\b/iu.test(tail))continue;
        const handoff=/^(?:轮到你了|到你了|准备一下|下一位|your turn|over to you)[.!！。\s]*$/iu.test(tail);
        if((!tail && !trailing)||handoff){this.until=now+12000;return {answer:false,alert:name};}
        if(trailing || request.test(tail) || (breaks.test(after) && /[?？]|为什么|如何|怎么|什么|是否|能否|呢[。.]?$/u.test(tail))){
          return {answer:true,alert:name};
        }
      }
    }
    return {answer:pending && aliases(setup.names).length>0 && request.test(text.replace(breaks,'')),alert:''};
  }
}
