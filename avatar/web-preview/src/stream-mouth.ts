// Mapping adapted from InterviewFlowAI/talking-avatar d3e4dcf (MIT).
// Copyright (c) 2026 InterviewFlowAI. See ../licenses/TalkingAvatar-MIT.txt.
// TingXian streaming envelope: fixed ramps, no duration-fraction or unbounded future.
import type { VisemeEvent } from "./vendor/visemeDetector";

const shapes = ["sil","aa","aa","O","E","E","I","U","O","aa","O","aa","kk","RR","nn","SS","CH","TH","FF","DD","kk","PP"];
const tongue = new Set([12,14,19,20]);
const heavy = new Set([7,8,10,16]);
const closure = new Set([18,21]);
export const BORROW_MS = 120;
export const ATTACK_MS = 95;
export const RELEASE_MS = 60;
export const MOUTH_FUTURE_MS = BORROW_MS + ATTACK_MS;
const smooth = (x:number) => { x=Math.max(0,Math.min(1,x)); return x*x*(3-2*x); };

/** Output at t is invariant to events strictly after t + MOUTH_FUTURE_MS.
 * Tongue-only phonemes borrow nearby lip shape, but within a fixed 120ms horizon.
 * Unlike offline coarticulation, an unknown segment end never changes past output. */
export function streamMouthAt(events:readonly VisemeEvent[],t:number) {
  const active:{shape:string;weight:number;level:number;closure:boolean}[]=[];
  let total=0,closed=0;
  for(let i=0;i<events.length;i++) {
    const event=events[i],end=events[i+1]?.ms??Infinity;
    if(event.ms>t+ATTACK_MS) break;
    if(end<=t-RELEASE_MS) continue;
    let source=i,level=1;
    if(tongue.has(event.id)) {
      let found=false;
      for(const direction of [1,-1]) {
        if(found) break;
        for(let j=i+direction;j>=0&&j<events.length;j+=direction) {
          const candidate=events[j];
          if(Math.abs(candidate.ms-event.ms)>BORROW_MS || candidate.id===0 || closure.has(candidate.id)) break;
          if(!tongue.has(candidate.id)) {source=j;found=true;break;}
        }
      }
      if(found) level=.25;
    }
    const id=events[source].id,attack=heavy.has(id)?ATTACK_MS:80;
    const weight=t<event.ms?smooth((t-event.ms+attack)/attack):t<=end?1:smooth(1-(t-end)/RELEASE_MS);
    if(weight<=0) continue;
    const isClosure=closure.has(id);
    active.push({shape:shapes[id]??"sil",weight,level,closure:isClosure});
    if(isClosure) closed=Math.max(closed,weight); else total+=weight;
  }
  const visemes:Record<string,number>={};
  for(const item of active) {
    if(item.shape==="sil") continue;
    const value=item.closure?item.weight*.9:item.weight*item.level*.78*(1-closed)/Math.max(1,total);
    if(value>0) visemes[item.shape]=item.closure?Math.max(visemes[item.shape]??0,value):(visemes[item.shape]??0)+value;
  }
  return visemes;
}
