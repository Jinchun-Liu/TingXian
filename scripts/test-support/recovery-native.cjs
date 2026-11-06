// Synthetic IPC persistence, shared by real-page regression fixtures.
module.exports=function recoveryNative(){
  let epoch=0;const drafts=new Map();
  const invoke=async(name,args)=>{
    if(name==='interview_recovery_list')return {epoch,drafts:[...drafts.values()].map(({draft,...row})=>structuredClone(row))};
    if(name==='interview_recovery_load'){if(!drafts.has(args.id))throw Error('draft missing');return structuredClone(drafts.get(args.id).draft);}
    if(name==='interview_recovery_write'){
      if(args.epoch!==epoch)throw Error('stale epoch');
      if((drafts.get(args.id)?.revision??0)>=args.revision)throw Error('stale revision');
      drafts.set(args.id,{id:args.id,revision:args.revision,updated_at:new Date().toISOString(),turns:args.draft.snapshot.history.length,calls:args.draft.snapshot.calls,draft:structuredClone(args.draft)});return;
    }
    throw Error('unknown recovery command '+name);
  };
  invoke.clear=()=>{epoch++;drafts.clear();};invoke.drafts=drafts;return invoke;
};
