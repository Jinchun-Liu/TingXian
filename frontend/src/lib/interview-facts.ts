export type FactSourceRef = {source_id:string;root:string;path:string;hash:string};
export type FactSource = FactSourceRef & {location:string;text:string;kind:string;user_assertion?:import('./interview-documents').UserAssertion|null};
export type FactCard = {fact_id:string;version:number;previous_version:number|null;text:string;source_note:string;sources:FactSource[];reason:string;confirmed_at:string|null;recorded_at:string};
export type FactCorrection = {expected_version:number;text:string;source_note:string;sources:FactSourceRef[];reason:string;confirmed:boolean};
export type FactHistory = {versions:FactCard[];next_before:number|null};
export type FactCandidate = {source_id:string;root?:string;path:string;hash?:string;location:string;text:string;kind:string;user_assertion?:import('./interview-documents').UserAssertion|null};
export function legacyFactCard(text:string):FactCard {
  return {fact_id:'personal-fact-card',version:0,previous_version:null,text,source_note:'旧版事实卡，未重新确认',sources:[],reason:'',confirmed_at:null,recorded_at:''};
}
